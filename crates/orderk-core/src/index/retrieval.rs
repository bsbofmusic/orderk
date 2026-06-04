//! Query/retrieval pipeline: routing, filter SQL, vector scoring, and the
//! hybrid/exact search execution paths.
//!
//! The DB-touching retrieval core: route-hit scoring, FTS/filter SQL
//! assembly, sqlite-vec and exact vector score collection, the
//! `query_hybrid` / `query_exact` execution paths, and `load_chunk_result`
//! row assembly. Calls into `evidence::build_result` and back-references
//! nothing in `index.rs`. Extracted from `index.rs`.

use super::evidence::build_result;
use super::query_plan::{QueryPlan, QueryRoute};
use super::ranking::sort_search_results;
use super::scoring::{
    blob_to_vec, bm25_to_score, distance_to_score, keyword_overlap_score, l2_distance,
    vector_to_blob,
};
use crate::embedding::EmbeddingProvider;
use crate::filter::FilterSql;
use crate::models::*;
use anyhow::Result;
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

#[derive(Debug, Clone)]
pub(crate) struct RouteHit {
    pub(crate) score: f32,
    pub(crate) reasons: Vec<String>,
}

pub(crate) fn clean_route_term(term: &str) -> String {
    term.trim()
        .trim_start_matches("path:")
        .trim_start_matches("tag:")
        .trim_start_matches('#')
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '(' || c == ')')
        .to_lowercase()
}

pub(crate) fn route_terms(plan: &QueryPlan) -> Vec<String> {
    let mut out = Vec::new();
    for pattern in &plan.patterns {
        let cleaned = clean_route_term(pattern);
        if !cleaned.is_empty() {
            out.push(cleaned);
        }
    }
    out.sort();
    out.dedup();
    out
}

pub(crate) fn add_reason(hit: &mut RouteHit, label: &str, score: f32) {
    if !hit.reasons.iter().any(|reason| reason == label) {
        hit.score += score;
        hit.reasons.push(label.to_string());
    }
}

pub(crate) fn score_route_hit(
    plan: &QueryPlan,
    path: &str,
    title: Option<&str>,
    heading: Option<&str>,
    tags_json: &str,
) -> Option<RouteHit> {
    let path_l = path.to_lowercase();
    let title_l = title.unwrap_or("").to_lowercase();
    let heading_l = heading.unwrap_or("").to_lowercase();
    let tags_l = tags_json.to_lowercase();
    let mut hit = RouteHit {
        score: 0.0,
        reasons: Vec::new(),
    };

    for term in route_terms(plan) {
        if term.is_empty() {
            continue;
        }
        if path_l.contains(&term) {
            add_reason(&mut hit, "path", if path_l == term { 0.18 } else { 0.12 });
        }
        if title_l.contains(&term) {
            add_reason(&mut hit, "title", 0.08);
        }
        if heading_l.contains(&term) {
            add_reason(&mut hit, "heading", 0.08);
        }
        if tags_l.contains(&term) {
            add_reason(&mut hit, "tag", 0.10);
        }
    }

    if matches!(plan.route, QueryRoute::Short) && hit.score == 0.0 {
        for term in &plan.terms {
            if path_l.contains(term) {
                add_reason(&mut hit, "path", 0.08);
            }
            if title_l.contains(term) {
                add_reason(&mut hit, "title", 0.06);
            }
            if heading_l.contains(term) {
                add_reason(&mut hit, "heading", 0.06);
            }
            if tags_l.contains(term) {
                add_reason(&mut hit, "tag", 0.08);
            }
        }
    }

    if hit.score > 0.0 {
        Some(hit)
    } else {
        None
    }
}

pub(crate) fn append_filter_clause(sql: &mut String, filter: Option<&FilterSql>) {
    if let Some(filter) = filter {
        sql.push_str(" AND ");
        sql.push_str(&filter.sql);
    }
}

pub(crate) fn append_filter_args(args: &mut Vec<Value>, filter: Option<&FilterSql>) {
    if let Some(filter) = filter {
        args.extend(filter.args.iter().cloned());
    }
}

pub(crate) fn collect_route_hits(
    conn: &Connection,
    plan: &QueryPlan,
    limit: usize,
    filter: Option<&FilterSql>,
) -> Result<HashMap<i64, RouteHit>> {
    let mut hits: HashMap<i64, RouteHit> = HashMap::new();
    let limit = (limit * 4).max(16) as i64;
    for pattern in route_terms(plan) {
        if pattern.is_empty() {
            continue;
        }
        let like = format!("%{}%", pattern);
        let mut sql = String::from(
            "SELECT c.id, c.file_path, c.title, c.heading, c.tags_json
             FROM chunks c
             WHERE (lower(c.file_path) LIKE ?
                OR lower(coalesce(c.title, '')) LIKE ?
                OR lower(coalesce(c.heading, '')) LIKE ?
                OR lower(c.tags_json) LIKE ?)",
        );
        append_filter_clause(&mut sql, filter);
        sql.push_str(" LIMIT ?");
        let mut args = vec![
            Value::Text(like.clone()),
            Value::Text(like.clone()),
            Value::Text(like.clone()),
            Value::Text(like),
        ];
        append_filter_args(&mut args, filter);
        args.push(Value::Integer(limit));
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args.iter()), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        for row in rows {
            let (rowid, path, title, heading, tags_json) = row?;
            if let Some(hit) = score_route_hit(
                plan,
                &path,
                title.as_deref(),
                heading.as_deref(),
                &tags_json,
            ) {
                hits.entry(rowid)
                    .and_modify(|existing| {
                        existing.score += hit.score;
                        for reason in &hit.reasons {
                            if !existing.reasons.iter().any(|current| current == reason) {
                                existing.reasons.push(reason.clone());
                            }
                        }
                    })
                    .or_insert(hit);
            }
        }
    }
    Ok(hits)
}

pub(crate) fn sqlite_vec_vector_scores(
    conn: &Connection,
    qvec: &[f32],
    limit: usize,
) -> Result<HashMap<i64, (usize, f32)>> {
    let mut scores = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT rowid, distance FROM vec_chunks WHERE embedding MATCH ?1 AND k = ?2 ORDER BY distance ASC",
    )?;
    let rows = stmt.query_map(params![vector_to_blob(qvec), (limit * 4) as i64], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, f32>(1)?))
    })?;
    for (rank, row) in rows.enumerate() {
        let (rowid, distance) = row?;
        scores.insert(rowid, (rank + 1, distance_to_score(distance)));
    }
    Ok(scores)
}

pub(crate) fn filtered_exact_vector_scores(
    conn: &Connection,
    qvec: &[f32],
    limit: usize,
    filter: &FilterSql,
) -> Result<HashMap<i64, (usize, f32)>> {
    let mut sql = String::from(
        "SELECT c.id, e.embedding
         FROM chunks c
         JOIN chunk_embeddings e ON e.chunk_id = c.chunk_id
         WHERE 1 = 1",
    );
    append_filter_clause(&mut sql, Some(filter));
    let mut args = Vec::new();
    append_filter_args(&mut args, Some(filter));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    let mut scored = Vec::new();
    for row in rows {
        let (rowid, blob) = row?;
        let vector = blob_to_vec(&blob);
        scored.push((rowid, l2_distance(qvec, &vector)));
    }
    scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut scores = HashMap::new();
    for (rank, (rowid, distance)) in scored.into_iter().take((limit * 4).max(16)).enumerate() {
        scores.insert(rowid, (rank + 1, distance_to_score(distance)));
    }
    Ok(scores)
}

pub(crate) fn query_hybrid<P: EmbeddingProvider + ?Sized>(
    conn: &Connection,
    query: &str,
    plan: &QueryPlan,
    limit: usize,
    provider: &P,
    filter: Option<&FilterSql>,
    rerank: bool,
) -> Result<(Vec<SearchResult>, QueryRoutingEvidence)> {
    let mut timings = QueryTimings::default();
    let scoring_query = plan.scoring_text();
    let keyword_started = Instant::now();
    let mut keyword_scores: HashMap<i64, (usize, f32)> = HashMap::new();
    if let Some(keyword_query) = plan.keyword_query() {
        let mut sql = String::from(
            "SELECT fts_chunks.rowid, bm25(fts_chunks) AS score
             FROM fts_chunks
             JOIN chunks c ON c.id = fts_chunks.rowid
             WHERE fts_chunks MATCH ?",
        );
        append_filter_clause(&mut sql, filter);
        sql.push_str(" ORDER BY score ASC LIMIT ?");
        let mut args = vec![Value::Text(keyword_query)];
        append_filter_args(&mut args, filter);
        args.push(Value::Integer((limit * 4) as i64));
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args.iter()), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f32>(1)?))
        })?;
        for (rank, row) in rows.enumerate() {
            let (rowid, bm25) = row?;
            keyword_scores.insert(rowid, (rank + 1, bm25_to_score(bm25)));
        }
    }
    timings.keyword_ms = keyword_started.elapsed().as_millis();

    let vector_started = Instant::now();
    let qvec = provider.embed_query(query)?;
    let vector_scores = if let Some(filter) = filter {
        filtered_exact_vector_scores(conn, &qvec, limit, filter)?
    } else {
        sqlite_vec_vector_scores(conn, &qvec, limit)?
    };
    timings.vector_ms = vector_started.elapsed().as_millis();

    let route_started = Instant::now();
    let route_scores = collect_route_hits(conn, plan, limit, filter)?;
    timings.route_ms = route_started.elapsed().as_millis();

    let merge_started = Instant::now();
    let mut candidate_ids: HashSet<i64> = keyword_scores.keys().copied().collect();
    candidate_ids.extend(vector_scores.keys().copied());
    candidate_ids.extend(route_scores.keys().copied());
    let merged_candidates = candidate_ids.len();

    let mut results = Vec::new();
    for rowid in candidate_ids {
        if let Some(result) = load_chunk_result(
            conn,
            rowid,
            keyword_scores.get(&rowid),
            vector_scores.get(&rowid),
            route_scores.get(&rowid),
            plan,
            &scoring_query,
            filter,
            rerank,
        )? {
            results.push(result);
        }
    }
    sort_search_results(&mut results);
    timings.merge_ms = merge_started.elapsed().as_millis();

    let routing = QueryRoutingEvidence {
        strategy: "hybrid".to_string(),
        route: plan.route.as_str().to_string(),
        routes_attempted: plan.routes_attempted(),
        embedding_profile_fingerprint: None,
        filter: None,
        filter_mode: None,
        filtered_candidates: None,
        min_score: None,
        threshold_filtered: None,
        context_chunks: 0,
        include_links: false,
        expand_links: 0,
        retrieval_depth: 0,
        query_expansion: false,
        query_expansion_terms: Vec::new(),
        external_reranker: false,
        keyword_candidates: keyword_scores.len(),
        vector_candidates: vector_scores.len(),
        route_candidates: route_scores.len(),
        link_candidates: 0,
        merged_candidates,
        returned: 0,
        timings,
    };
    Ok((results, routing))
}

pub(crate) fn query_exact<P: EmbeddingProvider + ?Sized>(
    conn: &Connection,
    query: &str,
    plan: &QueryPlan,
    _limit: usize,
    provider: &P,
    filter: Option<&FilterSql>,
    rerank: bool,
) -> Result<(Vec<SearchResult>, QueryRoutingEvidence)> {
    let mut timings = QueryTimings::default();
    let scoring_query = plan.scoring_text();
    let vector_started = Instant::now();
    let qvec = provider.embed_query(query)?;
    let mut sql = String::from(
        "SELECT c.id, c.chunk_id, c.file_path, c.title, c.heading, c.line_start, c.line_end, c.text, c.tags_json, c.mtime, e.embedding, f.mtime, f.hash, c.has_code, c.has_link, c.has_task_list, c.has_incomplete_tasks, c.confidence, c.status, c.source_type, c.valid_from, c.valid_until, c.supersedes, c.superseded_by, c.updated
         FROM chunks c
         JOIN chunk_embeddings e ON e.chunk_id = c.chunk_id
         LEFT JOIN files f ON f.path = c.file_path
         WHERE 1 = 1",
    );
    append_filter_clause(&mut sql, filter);
    let mut args = Vec::new();
    append_filter_args(&mut args, filter);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), |row| {
        let emb: Vec<u8> = row.get(10)?;
        let vec = blob_to_vec(&emb);
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, i64>(5)? as usize,
            row.get::<_, i64>(6)? as usize,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, i64>(9)?,
            vec,
            row.get::<_, Option<i64>>(11)?,
            row.get::<_, i64>(13)? != 0,
            row.get::<_, i64>(14)? != 0,
            row.get::<_, i64>(15)? != 0,
            row.get::<_, i64>(16)? != 0,
            row.get::<_, Option<String>>(17)?,
            row.get::<_, Option<String>>(18)?,
            row.get::<_, Option<String>>(19)?,
            row.get::<_, Option<String>>(20)?,
            row.get::<_, Option<String>>(21)?,
            row.get::<_, Option<String>>(22)?,
            row.get::<_, Option<String>>(23)?,
            row.get::<_, Option<String>>(24)?,
        ))
    })?;
    let mut scored = Vec::new();
    let mut route_matches = 0usize;
    for row in rows {
        let (
            rowid,
            chunk_id,
            path,
            title,
            heading,
            line_start,
            line_end,
            text,
            tags_json,
            mtime,
            vec,
            file_mtime,
            has_code,
            has_link,
            has_task_list,
            has_incomplete_tasks,
            confidence,
            status,
            source_type,
            valid_from,
            valid_until,
            supersedes,
            superseded_by,
            updated,
        ) = row?;
        let distance = l2_distance(&qvec, &vec);
        let vector_score = distance_to_score(distance);
        let keyword_score = keyword_overlap_score(&scoring_query, &text, &tags_json);
        let route_hit = score_route_hit(
            plan,
            &path,
            title.as_deref(),
            heading.as_deref(),
            &tags_json,
        );
        if route_hit.is_some() {
            route_matches += 1;
        }
        let result = build_result(
            rowid,
            &chunk_id,
            &path,
            title,
            heading,
            line_start,
            line_end,
            &text,
            &tags_json,
            file_mtime.or(Some(mtime)),
            None,
            None,
            keyword_score,
            vector_score,
            route_hit.as_ref(),
            plan,
            &scoring_query,
            has_code,
            has_link,
            has_task_list,
            has_incomplete_tasks,
            confidence,
            status,
            source_type,
            valid_from,
            valid_until,
            supersedes,
            superseded_by,
            updated,
            rerank,
        )?;
        scored.push(result);
    }
    sort_search_results(&mut scored);
    timings.vector_ms = vector_started.elapsed().as_millis();
    let total_candidates = scored.len();
    for (rank, result) in scored.iter_mut().enumerate() {
        result.evidence.vector_rank = Some(rank + 1);
        if !result
            .evidence
            .sources
            .iter()
            .any(|source| source == "vector")
        {
            result.evidence.sources.push("vector".to_string());
        }
    }
    let routing = QueryRoutingEvidence {
        strategy: "exact".to_string(),
        route: plan.route.as_str().to_string(),
        routes_attempted: plan.routes_attempted(),
        embedding_profile_fingerprint: None,
        filter: None,
        filter_mode: None,
        filtered_candidates: None,
        min_score: None,
        threshold_filtered: None,
        context_chunks: 0,
        include_links: false,
        expand_links: 0,
        retrieval_depth: 0,
        query_expansion: false,
        query_expansion_terms: Vec::new(),
        external_reranker: false,
        keyword_candidates: 0,
        vector_candidates: total_candidates,
        route_candidates: route_matches,
        link_candidates: 0,
        merged_candidates: total_candidates,
        returned: 0,
        timings,
    };
    Ok((scored, routing))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn load_chunk_result(
    conn: &Connection,
    rowid: i64,
    keyword: Option<&(usize, f32)>,
    vector: Option<&(usize, f32)>,
    route_hit: Option<&RouteHit>,
    plan: &QueryPlan,
    query: &str,
    filter: Option<&FilterSql>,
    rerank: bool,
) -> Result<Option<SearchResult>> {
    let mut sql = String::from(
        "SELECT c.chunk_id, c.file_path, c.title, c.heading, c.line_start, c.line_end, c.text, c.tags_json, c.mtime, f.mtime, c.has_code, c.has_link, c.has_task_list, c.has_incomplete_tasks, c.confidence, c.status, c.source_type, c.valid_from, c.valid_until, c.supersedes, c.superseded_by, c.updated
         FROM chunks c
         LEFT JOIN files f ON f.path = c.file_path
         WHERE c.id = ?",
    );
    append_filter_clause(&mut sql, filter);
    let mut args = vec![Value::Integer(rowid)];
    append_filter_args(&mut args, filter);
    let mut stmt = conn.prepare(&sql)?;
    let row = stmt
        .query_row(params_from_iter(args.iter()), |row| {
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
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, i64>(10)? != 0,
                row.get::<_, i64>(11)? != 0,
                row.get::<_, i64>(12)? != 0,
                row.get::<_, i64>(13)? != 0,
                row.get::<_, Option<String>>(14)?,
                row.get::<_, Option<String>>(15)?,
                row.get::<_, Option<String>>(16)?,
                row.get::<_, Option<String>>(17)?,
                row.get::<_, Option<String>>(18)?,
                row.get::<_, Option<String>>(19)?,
                row.get::<_, Option<String>>(20)?,
                row.get::<_, Option<String>>(21)?,
            ))
        })
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
        file_mtime,
        has_code,
        has_link,
        has_task_list,
        has_incomplete_tasks,
        confidence,
        status,
        source_type,
        valid_from,
        valid_until,
        supersedes,
        superseded_by,
        updated,
    )) = row
    else {
        return Ok(None);
    };
    build_result(
        rowid,
        &chunk_id,
        &path,
        title,
        heading,
        line_start,
        line_end,
        &text,
        &tags_json,
        file_mtime.or(Some(mtime)),
        keyword.map(|v| v.0),
        vector.map(|v| v.0),
        keyword.map(|v| v.1).unwrap_or(0.0),
        vector.map(|v| v.1).unwrap_or(0.0),
        route_hit,
        plan,
        query,
        has_code,
        has_link,
        has_task_list,
        has_incomplete_tasks,
        confidence,
        status,
        source_type,
        valid_from,
        valid_until,
        supersedes,
        superseded_by,
        updated,
        rerank,
    )
    .map(Some)
}
