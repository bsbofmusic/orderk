//! Result post-processing: scoring boosts, temporal-quality, and reranking.
//!
//! Pure functions over `SearchResult` / `ScoreBreakdown` with no SQLite
//! coupling: confidence/status/freshness/evidence boosts, validity windows,
//! temporal-quality application, quality/evidence summaries, result sorting,
//! and the model reranker. Extracted from `index.rs`.

use crate::models::*;
use crate::reranker::RerankerProvider;
use anyhow::{anyhow, Result};
use chrono::{NaiveDate, Utc};

pub(crate) fn parse_orderk_date(value: Option<&str>) -> Option<NaiveDate> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    NaiveDate::parse_from_str(value.get(..10).unwrap_or(value), "%Y-%m-%d").ok()
}

pub(crate) fn query_has_recent_cue(query: &str) -> bool {
    let q = query.to_lowercase();
    [
        "recent", "recently", "latest", "current", "today", "现在", "最新", "当前",
    ]
    .iter()
    .any(|cue| q.contains(cue))
}

pub(crate) fn query_has_oldest_cue(query: &str) -> bool {
    let q = query.to_lowercase();
    [
        "first",
        "earliest",
        "originally",
        "oldest",
        "最早",
        "原始",
        "历史",
    ]
    .iter()
    .any(|cue| q.contains(cue))
}

pub(crate) fn confidence_boost_value(confidence: Option<&str>) -> f32 {
    match confidence.unwrap_or("").trim().to_lowercase().as_str() {
        "verified" | "high" => 0.025,
        "observed" | "medium" => 0.012,
        "inferred" | "low" => -0.01,
        "stale" => -0.03,
        _ => 0.0,
    }
}

pub(crate) fn status_boost_value(status: Option<&str>, state: &str) -> f32 {
    if state == "stale" {
        return -0.04;
    }
    match status.unwrap_or("").trim().to_lowercase().as_str() {
        "active" | "current" | "valid" => 0.025,
        "draft" | "review" => 0.005,
        "stale" | "superseded" | "archived" | "deprecated" => -0.04,
        _ => 0.0,
    }
}

pub(crate) fn evidence_count_boost_value(result: &SearchResult) -> f32 {
    let count = result.evidence.sources.len().min(4) as f32;
    if count > 0.0 {
        0.005 * count
    } else {
        0.0
    }
}

pub(crate) fn freshness_boost_value(
    mode: &FreshnessMode,
    query: &str,
    age_days: Option<i64>,
    state: &str,
) -> f32 {
    if state == "stale" {
        return -0.02;
    }
    let Some(age_days) = age_days else {
        return 0.0;
    };
    match mode {
        FreshnessMode::Off => 0.0,
        FreshnessMode::Balanced => {
            if query_has_recent_cue(query) && age_days <= 30 {
                0.03
            } else if age_days <= 30 {
                0.01
            } else {
                0.0
            }
        }
        FreshnessMode::Recent => {
            if age_days <= 30 {
                0.05
            } else if age_days <= 180 {
                0.02
            } else {
                0.0
            }
        }
        FreshnessMode::Oldest => {
            if query_has_oldest_cue(query) || age_days >= 180 {
                0.03
            } else {
                0.0
            }
        }
    }
}

pub(crate) fn build_validity(result: &SearchResult, as_of: Option<NaiveDate>) -> ValidityEvidence {
    let reference_date = as_of.unwrap_or_else(|| Utc::now().date_naive());
    let valid_from = parse_orderk_date(result.valid_from.as_deref());
    let valid_until = parse_orderk_date(result.valid_until.as_deref());
    let updated = parse_orderk_date(result.updated.as_deref())
        .or_else(|| result.mtime.map(|mtime| mtime.date_naive()));
    let age_days = updated.map(|updated| (reference_date - updated).num_days().max(0));

    if let Some(start) = valid_from {
        if start > reference_date {
            return ValidityEvidence {
                state: "stale".to_string(),
                stale_reason: Some("not_yet_valid".to_string()),
                age_days,
                valid_from: result.valid_from.clone(),
                valid_until: result.valid_until.clone(),
                supersedes: result.supersedes.clone(),
                superseded_by: result.superseded_by.clone(),
                updated: result.updated.clone(),
            };
        }
    }
    let status_l = result.status.as_deref().unwrap_or("").trim().to_lowercase();
    if as_of.is_none()
        && matches!(
            status_l.as_str(),
            "stale" | "superseded" | "archived" | "deprecated"
        )
    {
        return ValidityEvidence {
            state: "stale".to_string(),
            stale_reason: Some(format!("status:{status_l}")),
            age_days,
            valid_from: result.valid_from.clone(),
            valid_until: result.valid_until.clone(),
            supersedes: result.supersedes.clone(),
            superseded_by: result.superseded_by.clone(),
            updated: result.updated.clone(),
        };
    }

    if let Some(end) = valid_until {
        if end < reference_date {
            return ValidityEvidence {
                state: "stale".to_string(),
                stale_reason: Some("valid_until".to_string()),
                age_days,
                valid_from: result.valid_from.clone(),
                valid_until: result.valid_until.clone(),
                supersedes: result.supersedes.clone(),
                superseded_by: result.superseded_by.clone(),
                updated: result.updated.clone(),
            };
        }
    }

    if as_of.is_some() {
        return ValidityEvidence {
            state: "historical".to_string(),
            stale_reason: None,
            age_days,
            valid_from: result.valid_from.clone(),
            valid_until: result.valid_until.clone(),
            supersedes: result.supersedes.clone(),
            superseded_by: result.superseded_by.clone(),
            updated: result.updated.clone(),
        };
    }
    if result
        .superseded_by
        .as_deref()
        .is_some_and(|v| !v.trim().is_empty())
    {
        return ValidityEvidence {
            state: "stale".to_string(),
            stale_reason: Some("superseded_by".to_string()),
            age_days,
            valid_from: result.valid_from.clone(),
            valid_until: result.valid_until.clone(),
            supersedes: result.supersedes.clone(),
            superseded_by: result.superseded_by.clone(),
            updated: result.updated.clone(),
        };
    }

    ValidityEvidence {
        state: "current".to_string(),
        stale_reason: None,
        age_days,
        valid_from: result.valid_from.clone(),
        valid_until: result.valid_until.clone(),
        supersedes: result.supersedes.clone(),
        superseded_by: result.superseded_by.clone(),
        updated: result.updated.clone(),
    }
}

pub(crate) fn result_has_temporal_quality_metadata(result: &SearchResult) -> bool {
    result
        .confidence
        .as_deref()
        .is_some_and(|v| !v.trim().is_empty())
        || result
            .status
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
        || result
            .source_type
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
        || result
            .valid_from
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
        || result
            .valid_until
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
        || result
            .supersedes
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
        || result
            .superseded_by
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
        || result
            .updated
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
}

pub(crate) fn apply_temporal_quality(
    results: &mut Vec<SearchResult>,
    options: &QueryOptions,
    query: &str,
) -> Result<()> {
    let as_of = match options
        .as_of
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(raw) => Some(
            parse_orderk_date(Some(raw))
                .ok_or_else(|| anyhow!("--as-of must use YYYY-MM-DD or RFC3339 date prefix"))?,
        ),
        None => None,
    };

    for result in results.iter_mut() {
        result.validity = build_validity(result, as_of);
        let has_temporal_quality = result_has_temporal_quality_metadata(result)
            || as_of.is_some()
            || query_has_recent_cue(query)
            || query_has_oldest_cue(query)
            || matches!(
                options.freshness,
                FreshnessMode::Recent | FreshnessMode::Oldest
            );
        if options.rerank && has_temporal_quality {
            let freshness = freshness_boost_value(
                &options.freshness,
                query,
                result.validity.age_days,
                &result.validity.state,
            );
            let confidence = confidence_boost_value(result.confidence.as_deref());
            let status = status_boost_value(result.status.as_deref(), &result.validity.state);
            let evidence = evidence_count_boost_value(result);
            result.score_breakdown.freshness_boost = freshness;
            result.score_breakdown.confidence_boost = confidence;
            result.score_breakdown.status_boost = status;
            result.score_breakdown.evidence_count_boost = evidence;
            result.score += freshness + confidence + status + evidence;
        }
        if has_temporal_quality
            && !result
                .evidence
                .sources
                .iter()
                .any(|source| source == "temporal")
        {
            result.evidence.sources.push("temporal".to_string());
        }
        refresh_evidence_count(result);
        refresh_result_summaries(result);
    }

    if !options.include_stale {
        results.retain(|result| result.validity.state != "stale");
    }
    sort_search_results(results);
    Ok(())
}

pub(crate) fn quality_summary(breakdown: &ScoreBreakdown) -> QualitySummary {
    let total_boost = breakdown.freshness_boost
        + breakdown.confidence_boost
        + breakdown.status_boost
        + breakdown.evidence_count_boost;
    QualitySummary {
        schema_version: "orderk.quality_summary.v1".to_string(),
        freshness_boost: breakdown.freshness_boost,
        confidence_boost: breakdown.confidence_boost,
        status_boost: breakdown.status_boost,
        evidence_count_boost: breakdown.evidence_count_boost,
        total_boost,
    }
}

pub(crate) fn evidence_summary(result: &SearchResult) -> EvidenceSummary {
    EvidenceSummary {
        schema_version: "orderk.evidence_summary.v1".to_string(),
        validity_state: result.validity.state.clone(),
        stale_reason: result.validity.stale_reason.clone(),
        age_days: result.validity.age_days,
        confidence: result.confidence.clone(),
        status: result.status.clone(),
        source_type: result.source_type.clone(),
        source_tier: result.source_tier.clone(),
        evidence_type: result.evidence_type.clone(),
        event_time: result.event_time.clone(),
        evidence_count: result.evidence.evidence_count,
        sources: result.evidence.sources.clone(),
        evidence_uri: result.evidence_uri.clone(),
        open_uri: result.open_uri.clone(),
    }
}

pub(crate) fn refresh_result_summaries(result: &mut SearchResult) {
    result.quality = quality_summary(&result.score_breakdown);
    result.evidence_summary = evidence_summary(result);
}

pub(crate) fn sort_search_results(results: &mut [SearchResult]) {
    results.sort_by(|a, b| {
        finite_score_for_sort(b.score)
            .partial_cmp(&finite_score_for_sort(a.score))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line_start.cmp(&b.line_start))
            .then_with(|| a.chunk_id.cmp(&b.chunk_id))
    });
}

pub(crate) fn apply_same_file_mmr(
    results: &mut Vec<SearchResult>,
    top_limit: usize,
    mmr_lambda: f32,
    diversity_escape_margin: f32,
    max_per_file: usize,
) {
    if results.len() <= 1 || top_limit <= 1 {
        return;
    }
    sort_search_results(results);
    let lambda = mmr_lambda.clamp(0.0, 1.0);
    let margin = diversity_escape_margin.max(0.0);
    let cap = max_per_file.max(1);
    let mut remaining = std::mem::take(results);
    let mut selected: Vec<SearchResult> = Vec::with_capacity(remaining.len());
    let mut per_file: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    while !remaining.is_empty() && selected.len() < top_limit {
        let best_relevance = remaining
            .iter()
            .map(|result| finite_score_for_sort(result.score))
            .fold(f32::NEG_INFINITY, f32::max);
        let selected_paths = selected
            .iter()
            .map(|result| result.path.as_str())
            .collect::<std::collections::HashSet<_>>();
        let mut best_idx = 0usize;
        let mut best_mmr = f32::NEG_INFINITY;
        for (idx, result) in remaining.iter().enumerate() {
            let relevance = finite_score_for_sort(result.score);
            let same_file_similarity = if selected_paths.contains(result.path.as_str()) {
                1.0
            } else {
                0.0
            };
            let outside_escape_margin = !selected.is_empty() && best_relevance - relevance > margin;
            let over_cap = per_file.get(&result.path).copied().unwrap_or(0) >= cap;
            let mut mmr_score = lambda * relevance - (1.0 - lambda) * same_file_similarity;
            if outside_escape_margin {
                mmr_score -= 1.0;
            }
            if over_cap {
                mmr_score -= 1.0;
            }
            if mmr_score > best_mmr
                || (mmr_score == best_mmr
                    && result.path < remaining[best_idx].path
                    && result.line_start <= remaining[best_idx].line_start)
            {
                best_mmr = mmr_score;
                best_idx = idx;
            }
        }
        let chosen = remaining.remove(best_idx);
        *per_file.entry(chosen.path.clone()).or_insert(0) += 1;
        selected.push(chosen);
    }
    selected.extend(remaining);
    *results = selected;
}

fn finite_score_for_sort(score: f32) -> f32 {
    if score.is_finite() {
        score
    } else {
        f32::NEG_INFINITY
    }
}

pub(crate) fn apply_model_reranker(
    results: &mut [SearchResult],
    query: &str,
    documents: &[String],
    reranker: &dyn RerankerProvider,
) -> Result<()> {
    if results.is_empty() {
        return Ok(());
    }
    if documents.len() != results.len() {
        return Err(anyhow!(
            "reranker document count mismatch: got {}, expected {}",
            documents.len(),
            results.len()
        ));
    }
    let scores = reranker.rerank(query, documents)?;
    if scores.len() != results.len() {
        return Err(anyhow!(
            "{} reranker returned {} scores for {} candidates",
            reranker.model_id(),
            scores.len(),
            results.len()
        ));
    }
    for (result, score) in results.iter_mut().zip(scores) {
        let normalized = score.clamp(0.0, 1.0);
        result.score_breakdown.reranker_boost = normalized;
        result.score = normalized + (result.score * 0.01);
        if !result
            .evidence
            .sources
            .iter()
            .any(|source| source == "qwen_reranker")
        {
            result.evidence.sources.push("qwen_reranker".to_string());
        }
        refresh_evidence_count(result);
        refresh_result_summaries(result);
    }
    sort_search_results(results);
    Ok(())
}

pub(crate) fn refresh_evidence_count(result: &mut SearchResult) {
    let link_count = result
        .evidence
        .links
        .as_ref()
        .map(|links| links.outgoing.len() + links.backlinks.len())
        .unwrap_or(0);
    result.evidence.evidence_count = result.evidence.sources.len() + link_count;
}

#[cfg(test)]
mod ranking_tests {
    use super::*;

    fn test_result(path: &str, score: f32) -> SearchResult {
        SearchResult {
            chunk_id: format!("{path}#1"),
            file_path: path.to_string(),
            path: path.to_string(),
            title: Some(path.to_string()),
            heading: None,
            line_start: 1,
            line_end: 1,
            evidence_uri: String::new(),
            open_uri: String::new(),
            snippet: path.to_string(),
            score,
            score_breakdown: Default::default(),
            evidence: Default::default(),
            quality: Default::default(),
            evidence_summary: Default::default(),
            context_chunks: Vec::new(),
            tags: Vec::new(),
            confidence: None,
            status: None,
            source_type: None,
            source_tier: None,
            evidence_type: None,
            event_time: None,
            validity: Default::default(),
            valid_from: None,
            valid_until: None,
            supersedes: None,
            superseded_by: None,
            updated: None,
            mtime: None,
        }
    }

    #[test]
    fn sort_search_results_demotes_non_finite_scores_below_finite_hits() {
        let mut results = vec![
            test_result("a-nan.md", f32::NAN),
            test_result("b-inf.md", f32::INFINITY),
            test_result("z-best-finite.md", 0.9),
        ];

        sort_search_results(&mut results);

        assert_eq!(
            results[0].path, "z-best-finite.md",
            "non-finite scores must never win top rank through fallback path ordering: {results:#?}"
        );
    }

    #[test]
    fn same_file_mmr_prefers_a_diverse_file_when_scores_are_close() {
        let mut results = vec![
            test_result("same.md", 1.00),
            SearchResult {
                chunk_id: "same.md#2".to_string(),
                line_start: 20,
                ..test_result("same.md", 0.99)
            },
            test_result("other.md", 0.94),
            test_result("far.md", 0.30),
        ];

        apply_same_file_mmr(&mut results, 3, 0.72, 0.12, 2);

        let top_paths = results
            .iter()
            .take(3)
            .map(|r| r.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(top_paths, vec!["same.md", "other.md", "same.md"]);
    }
}
