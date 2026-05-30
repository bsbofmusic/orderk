//! Pure scoring, snippet, and vector primitives.
//!
//! These functions have no `IndexStore`/SQLite coupling. They are leaf helpers
//! used by the retrieval pipeline: score fusion, boosts, snippet extraction, and
//! vector blob (de)serialization. Extracted from `index.rs` to keep the hot
//! retrieval path readable.

use chrono::Utc;

pub(crate) fn normalize_query(query: &str) -> String {
    query
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn bm25_to_score(v: f32) -> f32 {
    1.0 / (1.0 + v.abs())
}

pub(crate) fn distance_to_score(v: f32) -> f32 {
    1.0 / (1.0 + v.max(0.0))
}

pub(crate) fn path_boost(path: &str, query: &str) -> f32 {
    let path_l = path.to_lowercase();
    let q = query.to_lowercase();
    if q.split_whitespace().any(|term| path_l.contains(term)) {
        0.08
    } else {
        0.0
    }
}

pub(crate) fn tag_boost(tags: &[String], query: &str) -> f32 {
    let q = query.to_lowercase();
    if tags.iter().any(|t| q.contains(&t.to_lowercase())) {
        0.05
    } else {
        0.0
    }
}

pub(crate) fn recency_boost(mtime: Option<i64>) -> f32 {
    let Some(mtime) = mtime else {
        return 0.0;
    };
    let age_days = ((Utc::now().timestamp() - mtime) as f32 / 86_400.0).max(0.0);
    (1.0 / (1.0 + age_days / 30.0)) * 0.03
}

pub(crate) fn metadata_boost_score(
    has_code: bool,
    has_link: bool,
    has_task_list: bool,
    has_incomplete_tasks: bool,
    confidence: Option<&str>,
    status: Option<&str>,
    source_type: Option<&str>,
) -> f32 {
    let mut boost: f32 = 0.0;
    if has_code {
        boost += 0.02;
    }
    if has_task_list {
        boost += 0.02;
    }
    if has_incomplete_tasks {
        boost -= 0.02;
    }
    if has_link {
        boost += 0.01;
    }
    match normalized_metadata_value(confidence).as_deref() {
        Some("high") => boost += 0.03,
        Some("medium") => boost += 0.01,
        Some("low") => boost -= 0.03,
        _ => {}
    }
    match normalized_metadata_value(status).as_deref() {
        Some("active") => boost += 0.02,
        Some("mature") => boost += 0.02,
        Some("stale") => boost -= 0.02,
        Some("archived") => boost -= 0.04,
        Some("deprecated") => boost -= 0.05,
        _ => {}
    }
    match normalized_metadata_value(source_type).as_deref() {
        Some("source") | Some("original") | Some("audit") | Some("manual_audit") => boost += 0.01,
        _ => {}
    }
    boost.clamp(-0.08, 0.08)
}

pub(crate) fn normalized_metadata_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

pub(crate) fn reciprocal_rank_fusion(
    keyword_rank: Option<usize>,
    vector_rank: Option<usize>,
) -> f32 {
    const RRF_K: f32 = 60.0;
    let score = |rank: usize| 1.0 / (RRF_K + rank as f32);
    keyword_rank.map(score).unwrap_or(0.0) + vector_rank.map(score).unwrap_or(0.0)
}

pub(crate) fn snippet(text: &str, query: &str) -> String {
    const CONTEXT_BEFORE: usize = 30;
    const CONTEXT_AFTER: usize = 60;
    const MAX_WINDOWS: usize = 3;
    const FALLBACK_CHARS: usize = 180;

    let lower = text.to_lowercase();
    let total_chars = text.chars().count();
    let mut terms = normalize_query(query)
        .split_whitespace()
        .map(|term| term.to_lowercase())
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();

    let mut windows = Vec::new();
    for term in &terms {
        if let Some(pos) = lower.find(term) {
            let char_pos = lower[..pos].chars().count();
            let start = char_pos.saturating_sub(CONTEXT_BEFORE);
            let end = (char_pos + term.chars().count() + CONTEXT_AFTER).min(total_chars);
            windows.push((start, end));
        }
    }

    if windows.is_empty() {
        return clean_snippet_text(&text.chars().take(FALLBACK_CHARS).collect::<String>());
    }

    windows.sort_by_key(|(start, _)| *start);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in windows {
        if let Some((_, prev_end)) = merged.last_mut() {
            if start <= *prev_end + 12 {
                *prev_end = (*prev_end).max(end);
                continue;
            }
        }
        merged.push((start, end));
    }

    merged
        .into_iter()
        .take(MAX_WINDOWS)
        .map(|(start, end)| {
            clean_snippet_text(
                &text
                    .chars()
                    .skip(start)
                    .take(end.saturating_sub(start))
                    .collect::<String>(),
            )
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" … ")
}

pub(crate) fn clean_snippet_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn keyword_overlap_score(query: &str, text: &str, tags_json: &str) -> f32 {
    let tags: Vec<String> = serde_json::from_str(tags_json).unwrap_or_default();
    let q = query.to_lowercase();
    let mut score: f32 = 0.0;
    for term in q.split_whitespace() {
        if text.to_lowercase().contains(term) {
            score += 0.1;
        }
        if tags.iter().any(|t| t.to_lowercase().contains(term)) {
            score += 0.05;
        }
    }
    score.min(1.0)
}

pub(crate) fn vector_to_blob(v: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for x in v {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    bytes
}

pub(crate) fn blob_to_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

pub(crate) fn l2_distance(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f32>()
        .sqrt()
}
