//! Link-expansion, evidence enrichment, and context-chunk loading.
//!
//! The DB-touching retrieval tail: wikilink-based link expansion (outgoing /
//! backlink rowids), evidence summaries, context-chunk windows around a hit,
//! and `build_result` assembly. Calls back into `index::load_chunk_result`
//! and uses `index::RouteHit` via `super::`. Extracted from `index.rs`.

use super::links::{link_points_to, normalize_wikilink_target};
use super::query_plan::{QueryIntent, QueryPlan};
use super::ranking::{refresh_evidence_count, sort_search_results};
use super::retrieval::{load_chunk_result, RouteHit};
use super::scoring::{
    metadata_boost_score, path_boost, recency_boost, reciprocal_rank_fusion, snippet, tag_boost,
};
use super::uri::{evidence_uri, open_uri};
use crate::filter::FilterSql;
use crate::models::*;
use crate::source_intel::{infer_event_time, infer_evidence_type, infer_source_tier};
use anyhow::Result;
use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};

pub(crate) const LINK_EXPANSION_BOOST: f32 = 0.03;
pub(crate) const LINK_EXPANSION_SEED_LIMIT: usize = 10;

pub(crate) fn expand_link_candidates(
    conn: &Connection,
    results: &mut Vec<SearchResult>,
    limit: usize,
    plan: &QueryPlan,
    query: &str,
    filter: Option<&FilterSql>,
    rerank: bool,
) -> Result<usize> {
    if results.is_empty() || limit == 0 {
        return Ok(0);
    }

    let seeds = results
        .iter()
        .take(limit.min(LINK_EXPANSION_SEED_LIMIT))
        .cloned()
        .collect::<Vec<_>>();
    let mut candidate_rowids = HashSet::new();
    for seed in &seeds {
        for rowid in outgoing_link_rowids(conn, seed)? {
            candidate_rowids.insert(rowid);
        }
        for rowid in backlink_rowids(conn, seed)? {
            candidate_rowids.insert(rowid);
        }
    }
    let mut candidate_rowids = candidate_rowids.into_iter().collect::<Vec<_>>();
    candidate_rowids.sort_unstable();

    let mut by_chunk_id = results
        .iter()
        .enumerate()
        .map(|(idx, result)| (result.chunk_id.clone(), idx))
        .collect::<HashMap<_, _>>();
    let mut touched = 0usize;

    for rowid in candidate_rowids {
        let Some(mut candidate) =
            load_chunk_result(conn, rowid, None, None, None, plan, query, filter, rerank)?
        else {
            continue;
        };

        if let Some(idx) = by_chunk_id.get(&candidate.chunk_id).copied() {
            if apply_link_expansion_signal(&mut results[idx]) {
                touched += 1;
            }
        } else {
            apply_link_expansion_signal(&mut candidate);
            by_chunk_id.insert(candidate.chunk_id.clone(), results.len());
            results.push(candidate);
            touched += 1;
        }
    }

    sort_search_results(results);
    Ok(touched)
}

pub(crate) fn apply_link_expansion_signal(result: &mut SearchResult) -> bool {
    let mut changed = false;
    if result.evidence.retrieval_depth < 1 {
        result.evidence.retrieval_depth = 1;
        changed = true;
    }
    if !result
        .evidence
        .sources
        .iter()
        .any(|source| source == "link_expansion")
    {
        result.evidence.sources.push("link_expansion".to_string());
        changed = true;
    }
    if result.score_breakdown.link_boost < LINK_EXPANSION_BOOST {
        let delta = LINK_EXPANSION_BOOST - result.score_breakdown.link_boost;
        result.score_breakdown.link_boost = LINK_EXPANSION_BOOST;
        result.score += delta;
        changed = true;
    }
    refresh_evidence_count(result);
    changed
}

pub(crate) fn outgoing_link_rowids(conn: &Connection, seed: &SearchResult) -> Result<Vec<i64>> {
    let outgoing_json: Option<String> = conn
        .query_row(
            "SELECT links_json FROM chunks WHERE chunk_id = ?1 LIMIT 1",
            params![seed.chunk_id],
            |row| row.get(0),
        )
        .optional()?;
    let outgoing: Vec<String> = outgoing_json
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    if outgoing.is_empty() {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT id, file_path, title
         FROM chunks
         WHERE chunk_id != ?1",
    )?;
    let rows = stmt.query_map(params![seed.chunk_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut rowids = Vec::new();
    for row in rows {
        let (rowid, path, title) = row?;
        if outgoing
            .iter()
            .any(|link| link_points_to(link, &path, title.as_deref()))
        {
            rowids.push(rowid);
        }
    }
    rowids.sort_unstable();
    rowids.dedup();
    Ok(rowids)
}

pub(crate) fn backlink_rowids(conn: &Connection, seed: &SearchResult) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT id, links_json
         FROM chunks
         WHERE chunk_id != ?1 AND links_json != '[]'",
    )?;
    let rows = stmt.query_map(params![seed.chunk_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut rowids = Vec::new();
    for row in rows {
        let (rowid, links_json) = row?;
        let links: Vec<String> = serde_json::from_str(&links_json).unwrap_or_default();
        if links
            .iter()
            .any(|link| link_points_to(link, &seed.path, seed.title.as_deref()))
        {
            rowids.push(rowid);
        }
    }
    rowids.sort_unstable();
    rowids.dedup();
    Ok(rowids)
}

pub(crate) fn enrich_results(
    conn: &Connection,
    results: &mut [SearchResult],
    context_chunks: usize,
    include_links: bool,
) -> Result<()> {
    for result in results {
        if context_chunks > 0 {
            result.context_chunks = load_context_chunks(conn, result, context_chunks)?;
        }
        if include_links {
            let links = load_link_evidence(conn, result)?;
            if !links.outgoing.is_empty()
                && !result.evidence.sources.iter().any(|s| s == "wikilink")
            {
                result.evidence.sources.push("wikilink".to_string());
            }
            if !links.backlinks.is_empty()
                && !result.evidence.sources.iter().any(|s| s == "backlink")
            {
                result.evidence.sources.push("backlink".to_string());
            }
            result.evidence.links = Some(links);
        }
    }
    Ok(())
}

pub(crate) fn load_chunk_by_chunk_id(
    conn: &Connection,
    chunk_id: &str,
    detail: &ChunkGetDetail,
) -> Result<Option<ChunkGetResult>> {
    let row = conn
        .query_row(
            "SELECT c.chunk_id, c.file_path, c.title, c.heading, c.line_start, c.line_end, c.text, c.tags_json, c.mtime, c.confidence, c.status, c.source_type, c.valid_from, c.valid_until, c.supersedes, c.superseded_by, c.updated, f.mtime
             FROM chunks c
             LEFT JOIN files f ON f.path = c.file_path
             WHERE c.chunk_id = ?1
             LIMIT 1",
            params![chunk_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)? as usize,
                    row.get::<_, i64>(5)? as usize,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, Option<String>>(16)?,
                    row.get::<_, Option<i64>>(17)?,
                ))
            },
        )
        .optional()?;

    let Some((
        chunk_id,
        path,
        title,
        heading,
        line_start,
        line_end,
        text,
        tags_json,
        mtime,
        confidence,
        status,
        source_type,
        valid_from,
        valid_until,
        supersedes,
        superseded_by,
        updated,
        file_mtime,
    )) = row
    else {
        return Ok(None);
    };
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
    let text = match detail {
        ChunkGetDetail::Full => text,
        ChunkGetDetail::Summary => snippet(&text, ""),
    };
    let evidence_uri = evidence_uri(&chunk_id);
    let open_uri = open_uri(&path, line_start);
    let source_tier = infer_source_tier(&path, source_type.as_deref());
    let evidence_type = infer_evidence_type(&path, source_type.as_deref());
    let event_time = infer_event_time(
        &path,
        valid_from.as_deref(),
        updated.as_deref(),
        file_mtime.or(Some(mtime)),
    );
    Ok(Some(ChunkGetResult {
        chunk_id,
        path,
        title,
        heading,
        line_start,
        line_end,
        evidence_uri,
        open_uri,
        text,
        tags,
        confidence,
        status,
        source_type,
        source_tier: Some(source_tier),
        evidence_type: Some(evidence_type),
        event_time,
        valid_from,
        valid_until,
        supersedes,
        superseded_by,
        updated,
        mtime: file_mtime
            .or(Some(mtime))
            .and_then(|ts| Utc.timestamp_opt(ts, 0).single()),
        context_chunks: Vec::new(),
    }))
}

pub(crate) fn load_context_chunks_for_chunk(
    conn: &Connection,
    path: &str,
    line_start: usize,
    line_end: usize,
    radius: usize,
) -> Result<Vec<SearchContextChunk>> {
    let mut before_stmt = conn.prepare(
        "SELECT chunk_id, file_path, heading, line_start, line_end, text
         FROM chunks
         WHERE file_path = ?1 AND line_end < ?2
         ORDER BY line_end DESC
         LIMIT ?3",
    )?;
    let before_rows =
        before_stmt.query_map(params![path, line_start as i64, radius as i64], |row| {
            Ok(SearchContextChunk {
                relation: "before".to_string(),
                chunk_id: row.get(0)?,
                path: row.get(1)?,
                heading: row.get(2)?,
                line_start: row.get::<_, i64>(3)? as usize,
                line_end: row.get::<_, i64>(4)? as usize,
                text: row.get(5)?,
            })
        })?;
    let mut before = Vec::new();
    for row in before_rows {
        before.push(row?);
    }
    before.reverse();

    let mut after_stmt = conn.prepare(
        "SELECT chunk_id, file_path, heading, line_start, line_end, text
         FROM chunks
         WHERE file_path = ?1 AND line_start > ?2
         ORDER BY line_start ASC
         LIMIT ?3",
    )?;
    let after_rows =
        after_stmt.query_map(params![path, line_end as i64, radius as i64], |row| {
            Ok(SearchContextChunk {
                relation: "after".to_string(),
                chunk_id: row.get(0)?,
                path: row.get(1)?,
                heading: row.get(2)?,
                line_start: row.get::<_, i64>(3)? as usize,
                line_end: row.get::<_, i64>(4)? as usize,
                text: row.get(5)?,
            })
        })?;
    let mut context = before;
    for row in after_rows {
        context.push(row?);
    }
    Ok(context)
}

pub(crate) fn load_context_chunks(
    conn: &Connection,
    result: &SearchResult,
    radius: usize,
) -> Result<Vec<SearchContextChunk>> {
    let mut before_stmt = conn.prepare(
        "SELECT chunk_id, file_path, heading, line_start, line_end, text
         FROM chunks
         WHERE file_path = ?1 AND line_end < ?2
         ORDER BY line_end DESC
         LIMIT ?3",
    )?;
    let before_rows = before_stmt.query_map(
        params![result.path, result.line_start as i64, radius as i64],
        |row| {
            Ok(SearchContextChunk {
                relation: "before".to_string(),
                chunk_id: row.get(0)?,
                path: row.get(1)?,
                heading: row.get(2)?,
                line_start: row.get::<_, i64>(3)? as usize,
                line_end: row.get::<_, i64>(4)? as usize,
                text: row.get(5)?,
            })
        },
    )?;
    let mut before = Vec::new();
    for row in before_rows {
        before.push(row?);
    }
    before.reverse();

    let mut after_stmt = conn.prepare(
        "SELECT chunk_id, file_path, heading, line_start, line_end, text
         FROM chunks
         WHERE file_path = ?1 AND line_start > ?2
         ORDER BY line_start ASC
         LIMIT ?3",
    )?;
    let after_rows = after_stmt.query_map(
        params![result.path, result.line_end as i64, radius as i64],
        |row| {
            Ok(SearchContextChunk {
                relation: "after".to_string(),
                chunk_id: row.get(0)?,
                path: row.get(1)?,
                heading: row.get(2)?,
                line_start: row.get::<_, i64>(3)? as usize,
                line_end: row.get::<_, i64>(4)? as usize,
                text: row.get(5)?,
            })
        },
    )?;
    let mut context = before;
    for row in after_rows {
        context.push(row?);
    }
    Ok(context)
}

pub(crate) fn load_link_evidence(conn: &Connection, result: &SearchResult) -> Result<LinkEvidence> {
    let outgoing_json: Option<String> = conn
        .query_row(
            "SELECT links_json FROM chunks WHERE chunk_id = ?1 LIMIT 1",
            params![result.chunk_id],
            |row| row.get(0),
        )
        .optional()?;
    let outgoing_raw: Vec<String> = outgoing_json
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    let mut outgoing = outgoing_raw
        .into_iter()
        .map(|target| OutgoingLinkEvidence {
            normalized_target: normalize_wikilink_target(&target),
            target,
        })
        .collect::<Vec<_>>();
    outgoing.sort_by(|a, b| a.normalized_target.cmp(&b.normalized_target));
    outgoing.dedup_by(|a, b| a.normalized_target == b.normalized_target && a.target == b.target);

    let mut stmt = conn.prepare(
        "SELECT file_path, title, links_json
         FROM chunks
         WHERE chunk_id != ?1 AND links_json != '[]'",
    )?;
    let rows = stmt.query_map(params![result.chunk_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut backlinks = Vec::new();
    for row in rows {
        let (source_path, source_title, links_json) = row?;
        let links: Vec<String> = serde_json::from_str(&links_json).unwrap_or_default();
        for link in links {
            if link_points_to(&link, &result.path, result.title.as_deref()) {
                backlinks.push(BacklinkEvidence {
                    source_path: source_path.clone(),
                    source_title: source_title.clone(),
                    normalized_target: normalize_wikilink_target(&link),
                    target: link,
                });
            }
        }
    }
    backlinks.sort_by(|a, b| {
        a.source_path
            .cmp(&b.source_path)
            .then(a.target.cmp(&b.target))
    });
    backlinks.dedup_by(|a, b| a.source_path == b.source_path && a.target == b.target);

    Ok(LinkEvidence {
        outgoing,
        backlinks,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_result(
    _rowid: i64,
    chunk_id: &str,
    path: &str,
    title: Option<String>,
    heading: Option<String>,
    line_start: usize,
    line_end: usize,
    text: &str,
    tags_json: &str,
    mtime: Option<i64>,
    keyword_rank: Option<usize>,
    vector_rank: Option<usize>,
    keyword_score: f32,
    vector_score: f32,
    route_hit: Option<&RouteHit>,
    plan: &QueryPlan,
    query: &str,
    has_code: bool,
    has_link: bool,
    has_task_list: bool,
    has_incomplete_tasks: bool,
    confidence: Option<String>,
    status: Option<String>,
    source_type: Option<String>,
    valid_from: Option<String>,
    valid_until: Option<String>,
    supersedes: Option<String>,
    superseded_by: Option<String>,
    updated: Option<String>,
    rerank: bool,
) -> Result<SearchResult> {
    let tags: Vec<String> = serde_json::from_str(tags_json).unwrap_or_default();
    let route_boost = route_hit.map(|hit| hit.score).unwrap_or(0.0);
    let fusion = reciprocal_rank_fusion(keyword_rank, vector_rank);
    let metadata_boost = if rerank {
        metadata_boost_score(
            has_code,
            has_link,
            has_task_list,
            has_incomplete_tasks,
            confidence.as_deref(),
            status.as_deref(),
            source_type.as_deref(),
        )
    } else {
        0.0
    };
    let source_tier = infer_source_tier(path, source_type.as_deref());
    let evidence_type = infer_evidence_type(path, source_type.as_deref());
    let event_time = infer_event_time(path, valid_from.as_deref(), updated.as_deref(), mtime);
    let source_tier_boost = if rerank {
        source_tier_intent_boost(plan.intent, &source_tier)
    } else {
        0.0
    };
    let intent_boost = if rerank {
        evidence_type_intent_boost(plan.intent, &evidence_type)
    } else {
        0.0
    };
    let event_time_boost = if rerank && event_time.is_some() {
        event_time_intent_boost(plan.intent)
    } else {
        0.0
    };
    let breakdown = ScoreBreakdown {
        keyword: keyword_score,
        vector: vector_score,
        fusion,
        path_boost: path_boost(path, query),
        tag_boost: tag_boost(&tags, query),
        route_boost,
        recency_boost: recency_boost(mtime),
        metadata_boost,
        link_boost: 0.0,
        optimizer_adjustment: 0.0,
        freshness_boost: 0.0,
        confidence_boost: 0.0,
        status_boost: 0.0,
        evidence_count_boost: 0.0,
        reranker_boost: 0.0,
        source_tier_boost,
        intent_boost,
        event_time_boost,
    };
    let score = (keyword_score * 0.35)
        + (vector_score * 0.35)
        + fusion
        + breakdown.path_boost
        + breakdown.tag_boost
        + breakdown.route_boost
        + breakdown.recency_boost
        + metadata_boost
        + source_tier_boost
        + intent_boost
        + event_time_boost
        + breakdown.link_boost;
    let snippet = snippet(text, query);
    let mut sources = Vec::new();
    if keyword_rank.is_some() || keyword_score > 0.0 {
        sources.push("keyword".to_string());
    }
    if vector_rank.is_some() || vector_score > 0.0 {
        sources.push("vector".to_string());
    }
    if let Some(hit) = route_hit {
        for reason in &hit.reasons {
            if !sources.iter().any(|current| current == reason) {
                sources.push(reason.clone());
            }
        }
    }
    Ok(SearchResult {
        chunk_id: chunk_id.to_string(),
        file_path: path.to_string(),
        path: path.to_string(),
        title,
        heading,
        line_start,
        line_end,
        evidence_uri: evidence_uri(chunk_id),
        open_uri: open_uri(path, line_start),
        snippet,
        score,
        score_breakdown: breakdown,
        evidence: SearchResultEvidence {
            evidence_count: sources.len(),
            sources,
            keyword_rank,
            vector_rank,
            route: Some(plan.route.as_str().to_string()),
            route_score: route_boost,
            retrieval_depth: 0,
            links: None,
        },
        quality: QualitySummary::default(),
        evidence_summary: EvidenceSummary::default(),
        context_chunks: Vec::new(),
        tags,
        confidence,
        status,
        source_type,
        source_tier: Some(source_tier),
        evidence_type: Some(evidence_type),
        event_time,
        validity: ValidityEvidence::default(),
        valid_from,
        valid_until,
        supersedes,
        superseded_by,
        updated,
        mtime: mtime.and_then(|ts| Utc.timestamp_opt(ts, 0).single()),
    })
}

fn source_tier_intent_boost(intent: QueryIntent, tier: &str) -> f32 {
    match intent {
        QueryIntent::Historical => match tier {
            "transcript" | "report" | "system_snapshot" => 0.05,
            "raw" | "brain" => 0.03,
            _ => 0.0,
        },
        QueryIntent::Concept => match tier {
            "wiki" | "brain" => 0.04,
            _ => 0.0,
        },
        QueryIntent::Config => match tier {
            "report" | "system_snapshot" | "raw" | "brain" => 0.04,
            _ => 0.0,
        },
        QueryIntent::General => 0.0,
    }
}

fn evidence_type_intent_boost(intent: QueryIntent, evidence_type: &str) -> f32 {
    match intent {
        QueryIntent::Historical => match evidence_type {
            "dialogue" | "event_record" | "system_record" => 0.03,
            "source" | "memory" => 0.015,
            _ => 0.0,
        },
        QueryIntent::Concept => match evidence_type {
            "concept" | "memory" => 0.03,
            _ => 0.0,
        },
        QueryIntent::Config => match evidence_type {
            "event_record" | "system_record" | "source" | "memory" => 0.03,
            _ => 0.0,
        },
        QueryIntent::General => 0.0,
    }
}

fn event_time_intent_boost(intent: QueryIntent) -> f32 {
    match intent {
        QueryIntent::Historical => 0.04,
        QueryIntent::Config => 0.02,
        QueryIntent::Concept | QueryIntent::General => 0.0,
    }
}

#[cfg(test)]
mod evidence_tests {
    use super::*;

    fn historical_result(rerank: bool) -> SearchResult {
        let plan = QueryPlan::analyze("alpha 什么时候开始");
        build_result(
            1,
            "chk_hist",
            "raw/transcripts/hermes-sessions/2026/06/08/alpha.md",
            Some("Alpha log".to_string()),
            None,
            1,
            5,
            "alpha started on this date",
            "[]",
            None,
            Some(1),
            Some(1),
            0.2,
            0.2,
            None,
            &plan,
            "alpha 什么时候开始",
            false,
            false,
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            rerank,
        )
        .unwrap()
    }

    #[test]
    fn build_result_applies_intent_metadata_boosts_for_historical_event_evidence() {
        let result = historical_result(true);
        assert_eq!(result.source_tier.as_deref(), Some("transcript"));
        assert_eq!(result.evidence_type.as_deref(), Some("dialogue"));
        assert_eq!(result.event_time.as_deref(), Some("2026-06-08"));
        assert!(result.score_breakdown.source_tier_boost > 0.0);
        assert!(result.score_breakdown.intent_boost > 0.0);
        assert!(result.score_breakdown.event_time_boost > 0.0);

        let no_rerank = historical_result(false);
        assert_eq!(no_rerank.score_breakdown.source_tier_boost, 0.0);
        assert_eq!(no_rerank.score_breakdown.intent_boost, 0.0);
        assert_eq!(no_rerank.score_breakdown.event_time_boost, 0.0);
    }
}
