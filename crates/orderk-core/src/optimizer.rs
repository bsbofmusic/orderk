use crate::models::{
    OptimizeProposal, OptimizeResponse, OptimizerMetrics, OptimizerRuntimeConfig, OptimizerStatus,
    QueryResponse, SearchResult,
};
use anyhow::{anyhow, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const STATE_KEY: &str = "optimizer_state_json";
const DEFAULT_MIN_EVENTS: usize = 20;
const MIN_TEXT_ONLY_PENALTY: f32 = 0.65;
const MAX_TEXT_ONLY_PENALTY: f32 = 1.0;
const PENALTY_STEP: f32 = 0.03;
const MAX_DYNAMIC_STOPWORDS: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OptimizerState {
    text_only_penalty: f32,
    dynamic_stopwords: Vec<String>,
    consecutive_adjustments: usize,
    last_applied_event_id: i64,
    last_vector_confirmed_ratio: Option<f64>,
    previous: Option<OptimizerSnapshot>,
    last_action: Option<String>,
    last_applied_at: Option<String>,
    last_rollback_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OptimizerSnapshot {
    text_only_penalty: f32,
    dynamic_stopwords: Vec<String>,
    last_vector_confirmed_ratio: Option<f64>,
}

#[derive(Debug, Clone)]
struct OptimizerEvent {
    id: i64,
    returned: usize,
    text_only_results: usize,
    vector_confirmed_results: usize,
    weak_terms: Vec<String>,
}

impl Default for OptimizerState {
    fn default() -> Self {
        Self {
            text_only_penalty: MAX_TEXT_ONLY_PENALTY,
            dynamic_stopwords: Vec::new(),
            consecutive_adjustments: 0,
            last_applied_event_id: 0,
            last_vector_confirmed_ratio: None,
            previous: None,
            last_action: None,
            last_applied_at: None,
            last_rollback_at: None,
        }
    }
}

pub fn optimizer_disabled() -> bool {
    std::env::var("ORDERK_OPTIMIZER")
        .map(|value| {
            let value = value.trim().to_lowercase();
            matches!(value.as_str(), "0" | "false" | "off" | "disabled")
        })
        .unwrap_or(false)
}

pub fn ensure_optimizer_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS optimizer_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            query_id TEXT NOT NULL,
            query TEXT NOT NULL,
            route TEXT NOT NULL,
            mode TEXT NOT NULL,
            returned INTEGER NOT NULL,
            top_score REAL,
            top_sources TEXT NOT NULL,
            keyword_candidates INTEGER NOT NULL,
            vector_candidates INTEGER NOT NULL,
            merged_candidates INTEGER NOT NULL,
            text_only_results INTEGER NOT NULL,
            vector_confirmed_results INTEGER NOT NULL,
            weak_terms TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_optimizer_events_created_at ON optimizer_events(created_at);
        CREATE TABLE IF NOT EXISTS optimizer_state (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        "#,
    )?;
    Ok(())
}

pub fn load_runtime_config(conn: &Connection) -> Result<OptimizerRuntimeConfig> {
    if optimizer_disabled() {
        return Ok(OptimizerRuntimeConfig::default());
    }
    let state = load_state(conn)?;
    Ok(OptimizerRuntimeConfig {
        text_only_penalty: state
            .text_only_penalty
            .clamp(MIN_TEXT_ONLY_PENALTY, MAX_TEXT_ONLY_PENALTY),
        dynamic_stopwords: state.dynamic_stopwords,
    })
}

pub fn apply_runtime_adjustments(
    results: &mut [SearchResult],
    config: &OptimizerRuntimeConfig,
) -> usize {
    if config.text_only_penalty >= 0.999 {
        return 0;
    }
    let mut adjusted = 0usize;
    for result in results.iter_mut() {
        let has_keyword = result
            .evidence
            .sources
            .iter()
            .any(|source| source == "keyword");
        let has_vector = result
            .evidence
            .sources
            .iter()
            .any(|source| source == "vector");
        if has_keyword && !has_vector {
            let old_score = result.score;
            result.score *= config.text_only_penalty;
            result.score_breakdown.optimizer_adjustment += result.score - old_score;
            if !result
                .evidence
                .sources
                .iter()
                .any(|source| source == "optimizer_penalty")
            {
                result
                    .evidence
                    .sources
                    .push("optimizer_penalty".to_string());
            }
            adjusted += 1;
        }
    }
    adjusted
}

pub fn record_query_and_maybe_optimize(
    conn: &Connection,
    response: &QueryResponse,
) -> Result<OptimizerStatus> {
    if optimizer_disabled() {
        let mut status = status_from_state(conn, OptimizerState::default())
            .unwrap_or_else(|_| disabled_status("⚙️ 自优化引擎已由 ORDERK_OPTIMIZER=off 关闭"));
        status.enabled = false;
        status.message = "⚙️ 自优化引擎已由 ORDERK_OPTIMIZER=off 关闭".to_string();
        return Ok(status);
    }
    ensure_optimizer_schema(conn)?;
    insert_query_event(conn, response)?;
    let state = load_state(conn)?;
    let pending = pending_event_count(conn, state.last_applied_event_id)?;
    if pending >= DEFAULT_MIN_EVENTS {
        let _ = apply_optimizer(conn, DEFAULT_MIN_EVENTS)?;
    }
    optimizer_status(conn)
}

pub fn optimizer_status(conn: &Connection) -> Result<OptimizerStatus> {
    if !optimizer_schema_exists(conn)? {
        return Ok(status_from_counts(OptimizerState::default(), 0, 0));
    }
    status_from_state(conn, load_state(conn)?)
}

pub fn dry_run_optimizer(conn: &Connection, min_events: usize) -> Result<OptimizeResponse> {
    if !optimizer_schema_exists(conn)? {
        let state = OptimizerState::default();
        return Ok(OptimizeResponse {
            schema_version: "orderk.optimize.v1".to_string(),
            ok: true,
            mode: "dry_run".to_string(),
            status: status_from_counts(state, 0, 0),
            proposal: Some(empty_proposal(min_events.max(1), 0)),
        });
    }
    let state = load_state(conn)?;
    let proposal = build_proposal(conn, &state, min_events.max(1))?;
    Ok(OptimizeResponse {
        schema_version: "orderk.optimize.v1".to_string(),
        ok: true,
        mode: "dry_run".to_string(),
        status: status_from_state(conn, state)?,
        proposal: Some(proposal),
    })
}

pub fn apply_optimizer(conn: &Connection, min_events: usize) -> Result<OptimizeResponse> {
    ensure_optimizer_schema(conn)?;
    let mut state = load_state(conn)?;
    let proposal = build_proposal(conn, &state, min_events.max(1))?;
    if proposal.eligible {
        let should_rollback = state.consecutive_adjustments >= 3
            && state
                .last_vector_confirmed_ratio
                .is_some_and(|previous| proposal.metrics.vector_confirmed_ratio + 0.05 < previous)
            && state.previous.is_some();
        if should_rollback {
            if let Some(previous) = state.previous.take() {
                state.text_only_penalty = previous.text_only_penalty;
                state.dynamic_stopwords = previous.dynamic_stopwords;
                state.last_vector_confirmed_ratio = previous.last_vector_confirmed_ratio;
                state.consecutive_adjustments = 0;
                state.last_applied_event_id = proposal.latest_event_id;
                state.last_rollback_at = Some(Utc::now().to_rfc3339());
                state.last_action = Some("rollback".to_string());
            }
        } else {
            state.previous = Some(OptimizerSnapshot {
                text_only_penalty: state.text_only_penalty,
                dynamic_stopwords: state.dynamic_stopwords.clone(),
                last_vector_confirmed_ratio: state.last_vector_confirmed_ratio,
            });
            if let Some(next) = proposal.text_only_penalty_to {
                state.text_only_penalty =
                    (next as f32).clamp(MIN_TEXT_ONLY_PENALTY, MAX_TEXT_ONLY_PENALTY);
            }
            for term in &proposal.stopwords_to_add {
                if !state
                    .dynamic_stopwords
                    .iter()
                    .any(|current| current == term)
                {
                    state.dynamic_stopwords.push(term.clone());
                }
            }
            state.dynamic_stopwords.sort();
            state.dynamic_stopwords.dedup();
            state.consecutive_adjustments += 1;
            state.last_vector_confirmed_ratio = Some(proposal.metrics.vector_confirmed_ratio);
            state.last_applied_event_id = proposal.latest_event_id;
            state.last_applied_at = Some(Utc::now().to_rfc3339());
            state.last_action = Some("apply".to_string());
        }
        save_state(conn, &state)?;
    }
    Ok(OptimizeResponse {
        schema_version: "orderk.optimize.v1".to_string(),
        ok: true,
        mode: "apply".to_string(),
        status: status_from_state(conn, load_state(conn)?)?,
        proposal: Some(proposal),
    })
}

pub fn reset_optimizer(conn: &Connection) -> Result<OptimizeResponse> {
    ensure_optimizer_schema(conn)?;
    let state = OptimizerState::default();
    save_state(conn, &state)?;
    Ok(OptimizeResponse {
        schema_version: "orderk.optimize.v1".to_string(),
        ok: true,
        mode: "reset".to_string(),
        status: status_from_state(conn, state)?,
        proposal: None,
    })
}

pub fn set_optimizer(
    conn: &Connection,
    text_only_penalty: Option<f64>,
    add_stopwords: &[String],
    remove_stopwords: &[String],
) -> Result<OptimizeResponse> {
    ensure_optimizer_schema(conn)?;
    if text_only_penalty.is_none() && add_stopwords.is_empty() && remove_stopwords.is_empty() {
        return Err(anyhow!(
            "optimize set requires --text-only-penalty, --add-stopword, or --remove-stopword"
        ));
    }

    let mut state = load_state(conn)?;
    if let Some(penalty) = text_only_penalty {
        if !penalty.is_finite()
            || penalty < MIN_TEXT_ONLY_PENALTY as f64
            || penalty > MAX_TEXT_ONLY_PENALTY as f64
        {
            return Err(anyhow!(
                "--text-only-penalty must be between {:.2} and {:.2}",
                MIN_TEXT_ONLY_PENALTY,
                MAX_TEXT_ONLY_PENALTY
            ));
        }
        state.text_only_penalty = penalty as f32;
    }

    for term in remove_stopwords {
        let term = normalize_manual_stopword(term)?;
        state.dynamic_stopwords.retain(|current| current != &term);
    }
    for term in add_stopwords {
        let term = normalize_manual_stopword(term)?;
        if !state.dynamic_stopwords.iter().any(|current| current == &term) {
            state.dynamic_stopwords.push(term);
        }
    }
    normalize_state(&mut state);
    state.last_action = Some("manual_set".to_string());
    save_state(conn, &state)?;

    Ok(OptimizeResponse {
        schema_version: "orderk.optimize.v1".to_string(),
        ok: true,
        mode: "set".to_string(),
        status: status_from_state(conn, state)?,
        proposal: None,
    })
}

fn insert_query_event(conn: &Connection, response: &QueryResponse) -> Result<()> {
    let top = response.results.first();
    let top_score = top.map(|result| result.score as f64);
    let top_sources = top
        .map(|result| serde_json::to_string(&result.evidence.sources))
        .transpose()?
        .unwrap_or_else(|| "[]".to_string());
    let text_only_results = response
        .results
        .iter()
        .filter(|result| {
            let has_keyword = result
                .evidence
                .sources
                .iter()
                .any(|source| source == "keyword");
            let has_vector = result
                .evidence
                .sources
                .iter()
                .any(|source| source == "vector");
            has_keyword && !has_vector
        })
        .count();
    let vector_confirmed_results = response
        .results
        .iter()
        .filter(|result| {
            result
                .evidence
                .sources
                .iter()
                .any(|source| source == "vector")
        })
        .count();
    let weak_terms = serde_json::to_string(&weak_terms_for_query(&response.query))?;
    conn.execute(
        "INSERT INTO optimizer_events(query_id, query, route, mode, returned, top_score, top_sources, keyword_candidates, vector_candidates, merged_candidates, text_only_results, vector_confirmed_results, weak_terms, created_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            response.query_id,
            response.query,
            response.route,
            response.mode,
            response.results.len() as i64,
            top_score,
            top_sources,
            response.routing.keyword_candidates as i64,
            response.routing.vector_candidates as i64,
            response.routing.merged_candidates as i64,
            text_only_results as i64,
            vector_confirmed_results as i64,
            weak_terms,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn build_proposal(
    conn: &Connection,
    state: &OptimizerState,
    min_events: usize,
) -> Result<OptimizeProposal> {
    let events = recent_events(conn, state.last_applied_event_id)?;
    let latest_event_id = events
        .last()
        .map(|event| event.id)
        .unwrap_or(state.last_applied_event_id);
    let metrics = metrics_for_events(&events);
    if events.len() < min_events {
        return Ok(OptimizeProposal {
            schema_version: "orderk.optimizer_proposal.v1".to_string(),
            eligible: false,
            reason: format!(
                "need {min_events} pending query events, currently {}",
                events.len()
            ),
            stopwords_to_add: Vec::new(),
            text_only_penalty_from: Some(state.text_only_penalty as f64),
            text_only_penalty_to: None,
            latest_event_id,
            metrics,
        });
    }

    let mut weak_counts: HashMap<String, usize> = HashMap::new();
    for event in &events {
        for term in &event.weak_terms {
            *weak_counts.entry(term.clone()).or_insert(0) += 1;
        }
    }
    let weak_threshold = min_events.min(3).max(1);
    let mut stopwords_to_add = weak_counts
        .into_iter()
        .filter(|(term, count)| {
            *count >= weak_threshold
                && !state
                    .dynamic_stopwords
                    .iter()
                    .any(|current| current == term)
        })
        .collect::<Vec<_>>();
    stopwords_to_add.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let remaining_stopword_slots =
        MAX_DYNAMIC_STOPWORDS.saturating_sub(state.dynamic_stopwords.len());
    let stopwords_to_add = stopwords_to_add
        .into_iter()
        .take(remaining_stopword_slots)
        .map(|(term, _)| term)
        .collect::<Vec<_>>();

    let text_only_penalty_to = if metrics.returned_results > 0
        && metrics.text_only_ratio > 0.25
        && state.text_only_penalty > MIN_TEXT_ONLY_PENALTY
    {
        Some(((state.text_only_penalty - PENALTY_STEP).max(MIN_TEXT_ONLY_PENALTY)) as f64)
    } else if metrics.returned_results > 0
        && metrics.text_only_ratio < 0.05
        && state.text_only_penalty < MAX_TEXT_ONLY_PENALTY
    {
        Some(((state.text_only_penalty + PENALTY_STEP).min(MAX_TEXT_ONLY_PENALTY)) as f64)
    } else {
        None
    };

    let eligible = !stopwords_to_add.is_empty() || text_only_penalty_to.is_some();
    let reason = if eligible {
        "bounded optimizer proposal ready".to_string()
    } else {
        "no bounded adjustment needed".to_string()
    };
    Ok(OptimizeProposal {
        schema_version: "orderk.optimizer_proposal.v1".to_string(),
        eligible,
        reason,
        stopwords_to_add,
        text_only_penalty_from: Some(state.text_only_penalty as f64),
        text_only_penalty_to,
        latest_event_id,
        metrics,
    })
}

fn metrics_for_events(events: &[OptimizerEvent]) -> OptimizerMetrics {
    let returned_results = events.iter().map(|event| event.returned).sum::<usize>();
    let text_only_results = events
        .iter()
        .map(|event| event.text_only_results)
        .sum::<usize>();
    let vector_confirmed_results = events
        .iter()
        .map(|event| event.vector_confirmed_results)
        .sum::<usize>();
    let text_only_ratio = if returned_results > 0 {
        text_only_results as f64 / returned_results as f64
    } else {
        0.0
    };
    let vector_confirmed_ratio = if returned_results > 0 {
        vector_confirmed_results as f64 / returned_results as f64
    } else {
        0.0
    };
    OptimizerMetrics {
        events: events.len(),
        returned_results,
        text_only_results,
        vector_confirmed_results,
        text_only_ratio,
        vector_confirmed_ratio,
    }
}

fn recent_events(conn: &Connection, after_id: i64) -> Result<Vec<OptimizerEvent>> {
    let mut stmt = conn.prepare(
        "SELECT id, returned, text_only_results, vector_confirmed_results, weak_terms
         FROM optimizer_events
         WHERE id > ?1
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map(params![after_id], |row| {
        let weak_terms_raw: String = row.get(4)?;
        Ok(OptimizerEvent {
            id: row.get(0)?,
            returned: row.get::<_, i64>(1)?.max(0) as usize,
            text_only_results: row.get::<_, i64>(2)?.max(0) as usize,
            vector_confirmed_results: row.get::<_, i64>(3)?.max(0) as usize,
            weak_terms: serde_json::from_str(&weak_terms_raw).unwrap_or_default(),
        })
    })?;
    let mut events = Vec::new();
    for row in rows {
        events.push(row?);
    }
    Ok(events)
}

fn status_from_state(conn: &Connection, state: OptimizerState) -> Result<OptimizerStatus> {
    let total_events = conn.query_row("SELECT COUNT(*) FROM optimizer_events", [], |row| {
        row.get::<_, i64>(0)
    })?;
    let pending_events = pending_event_count(conn, state.last_applied_event_id)?;
    Ok(status_from_counts(
        state,
        total_events.max(0) as usize,
        pending_events,
    ))
}

fn status_from_counts(
    state: OptimizerState,
    total_events: usize,
    pending_events: usize,
) -> OptimizerStatus {
    OptimizerStatus {
        schema_version: "orderk.optimizer_status.v1".to_string(),
        enabled: !optimizer_disabled(),
        message: optimizer_message(
            None,
            total_events,
            pending_events,
            state.text_only_penalty,
            state.dynamic_stopwords.len(),
        ),
        total_events,
        pending_events,
        text_only_penalty: state.text_only_penalty as f64,
        dynamic_stopwords: state.dynamic_stopwords,
        consecutive_adjustments: state.consecutive_adjustments,
        last_action: state.last_action,
        last_applied_at: state.last_applied_at,
        last_rollback_at: state.last_rollback_at,
    }
}

fn empty_proposal(min_events: usize, latest_event_id: i64) -> OptimizeProposal {
    OptimizeProposal {
        schema_version: "orderk.optimizer_proposal.v1".to_string(),
        eligible: false,
        reason: format!("need {min_events} pending query events, currently 0"),
        stopwords_to_add: Vec::new(),
        text_only_penalty_from: Some(MAX_TEXT_ONLY_PENALTY as f64),
        text_only_penalty_to: None,
        latest_event_id,
        metrics: OptimizerMetrics::default(),
    }
}

fn optimizer_schema_exists(conn: &Connection) -> Result<bool> {
    let count = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('optimizer_events', 'optimizer_state')",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(count >= 2)
}

pub fn disabled_optimizer_status() -> OptimizerStatus {
    disabled_status("⚙️ 自优化引擎已由 ORDERK_OPTIMIZER=off 关闭")
}

pub fn with_model_hint(mut status: OptimizerStatus, embedding_model: &str) -> OptimizerStatus {
    status.message = optimizer_message(
        Some(embedding_model),
        status.total_events,
        status.pending_events,
        status.text_only_penalty as f32,
        status.dynamic_stopwords.len(),
    );
    status
}

fn optimizer_message(
    embedding_model: Option<&str>,
    total_events: usize,
    pending_events: usize,
    text_only_penalty: f32,
    dynamic_stopwords: usize,
) -> String {
    let model_hint = embedding_model
        .map(|model| format!("当前模型 `{model}` "))
        .unwrap_or_default();
    format!(
        "⚙️ {model_hint}正在使用持续优化迭代算法优化搜索结果；如结果不满意，可用 `orderk optimize set --db <orderk.sqlite> --text-only-penalty <0.65-1.0> --add-stopword <term> --remove-stopword <term>` 手动调整参数来优化目标结果。已记录 {total_events} 次查询，待优化事件 {pending_events} 个；text_only_penalty={text_only_penalty:.2}，动态停用词 {dynamic_stopwords} 个。"
    )
}

fn normalize_manual_stopword(term: &str) -> Result<String> {
    let term = term.trim().to_lowercase();
    if term.is_empty() {
        Err(anyhow!("stopword cannot be empty"))
    } else {
        Ok(term)
    }
}

fn disabled_status(message: &str) -> OptimizerStatus {
    OptimizerStatus {
        schema_version: "orderk.optimizer_status.v1".to_string(),
        enabled: false,
        message: message.to_string(),
        total_events: 0,
        pending_events: 0,
        text_only_penalty: MAX_TEXT_ONLY_PENALTY as f64,
        dynamic_stopwords: Vec::new(),
        consecutive_adjustments: 0,
        last_action: Some("disabled".to_string()),
        last_applied_at: None,
        last_rollback_at: None,
    }
}

fn pending_event_count(conn: &Connection, after_id: i64) -> Result<usize> {
    Ok(conn
        .query_row(
            "SELECT COUNT(*) FROM optimizer_events WHERE id > ?1",
            params![after_id],
            |row| row.get::<_, i64>(0),
        )?
        .max(0) as usize)
}

fn load_state(conn: &Connection) -> Result<OptimizerState> {
    let raw = conn
        .query_row(
            "SELECT value FROM optimizer_state WHERE key = ?1",
            params![STATE_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(raw) = raw else {
        return Ok(OptimizerState::default());
    };
    let mut state = serde_json::from_str(&raw).unwrap_or_default();
    normalize_state(&mut state);
    Ok(state)
}

fn normalize_state(state: &mut OptimizerState) {
    state.dynamic_stopwords.sort();
    state.dynamic_stopwords.dedup();
    state.dynamic_stopwords.truncate(MAX_DYNAMIC_STOPWORDS);
}

fn save_state(conn: &Connection, state: &OptimizerState) -> Result<()> {
    conn.execute(
        "INSERT INTO optimizer_state(key, value, updated_at) VALUES(?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![
            STATE_KEY,
            serde_json::to_string(state)?,
            Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

pub(crate) fn filter_dynamic_stopwords(
    mut terms: Vec<String>,
    dynamic_stopwords: &[String],
) -> Vec<String> {
    if terms.len() <= 1 || dynamic_stopwords.is_empty() {
        return terms;
    }
    let before = terms.clone();
    terms.retain(|term| !dynamic_stopwords.iter().any(|stop| stop == term));
    if terms.is_empty() {
        before
    } else {
        terms
    }
}

fn weak_terms_for_query(query: &str) -> Vec<String> {
    let lower = query.to_lowercase();
    let mut terms = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|term| !term.trim().is_empty())
        .filter(|term| is_weak_term(term))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    for candidate in CHINESE_WEAK_TERMS {
        if query.contains(candidate) {
            terms.push((*candidate).to_string());
        }
    }
    terms.sort();
    terms.dedup();
    terms
}

fn is_weak_term(term: &str) -> bool {
    matches!(
        term,
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "be"
            | "can"
            | "do"
            | "does"
            | "for"
            | "from"
            | "how"
            | "in"
            | "is"
            | "of"
            | "on"
            | "or"
            | "the"
            | "to"
            | "what"
            | "why"
            | "with"
    )
}

const CHINESE_WEAK_TERMS: &[&str] = &[
    "如何",
    "怎么",
    "什么",
    "为什么",
    "一个",
    "一种",
    "这个",
    "那个",
    "自己",
    "我们",
    "你",
    "我",
    "他",
    "她",
    "它",
    "的",
    "了",
    "和",
    "与",
    "在",
    "是",
    "有",
    "让",
    "把",
    "被",
    "给",
    "对",
    "从",
    "到",
    "中",
    "里",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ScoreBreakdown, SearchResult, SearchResultEvidence};

    fn sample_result(sources: Vec<&str>, score: f32) -> SearchResult {
        SearchResult {
            chunk_id: "chk".to_string(),
            file_path: "note.md".to_string(),
            path: "note.md".to_string(),
            title: None,
            heading: None,
            line_start: 1,
            line_end: 1,
            evidence_uri: String::new(),
            open_uri: String::new(),
            snippet: "sample".to_string(),
            score,
            score_breakdown: ScoreBreakdown::default(),
            evidence: SearchResultEvidence {
                sources: sources.into_iter().map(ToString::to_string).collect(),
                ..SearchResultEvidence::default()
            },
            quality: Default::default(),
            evidence_summary: Default::default(),
            context_chunks: Vec::new(),
            tags: Vec::new(),
            confidence: None,
            status: None,
            source_type: None,
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
    fn dynamic_stopwords_never_remove_the_last_query_term() {
        let filtered = filter_dynamic_stopwords(
            vec!["how".to_string()],
            &["how".to_string(), "what".to_string()],
        );
        assert_eq!(filtered, vec!["how".to_string()]);

        let filtered = filter_dynamic_stopwords(
            vec!["how".to_string(), "money".to_string()],
            &["how".to_string()],
        );
        assert_eq!(filtered, vec!["money".to_string()]);
    }

    #[test]
    fn runtime_adjustment_penalizes_text_only_keyword_results() {
        let mut results = vec![
            sample_result(vec!["keyword"], 1.0),
            sample_result(vec!["keyword", "vector"], 1.0),
            sample_result(vec!["vector"], 1.0),
        ];
        let adjusted = apply_runtime_adjustments(
            &mut results,
            &OptimizerRuntimeConfig {
                text_only_penalty: 0.7,
                dynamic_stopwords: Vec::new(),
            },
        );
        assert_eq!(adjusted, 1);
        assert!((results[0].score - 0.7).abs() < f32::EPSILON);
        assert!((results[1].score - 1.0).abs() < f32::EPSILON);
        assert!(results[0]
            .evidence
            .sources
            .iter()
            .any(|source| source == "optimizer_penalty"));
    }

    #[test]
    fn apply_optimizer_adds_at_most_three_stopwords_and_steps_penalty() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_optimizer_schema(&conn).unwrap();
        for idx in 0..4 {
            conn.execute(
                "INSERT INTO optimizer_events(query_id, query, route, mode, returned, top_score, top_sources, keyword_candidates, vector_candidates, merged_candidates, text_only_results, vector_confirmed_results, weak_terms, created_at)
                 VALUES(?1, ?2, 'semantic', 'hybrid', 4, 1.0, '[]', 4, 1, 5, 2, 1, ?3, ?4)",
                params![
                    format!("q{idx}"),
                    "how what why to make money",
                    serde_json::to_string(&vec!["how", "what", "why", "to"]).unwrap(),
                    Utc::now().to_rfc3339(),
                ],
            )
            .unwrap();
        }
        let applied = apply_optimizer(&conn, 1).unwrap();
        let status = applied.status;
        assert!(status.dynamic_stopwords.len() <= 3);
        assert!((status.text_only_penalty - 0.97).abs() < 0.0001);

        for idx in 4..8 {
            conn.execute(
                "INSERT INTO optimizer_events(query_id, query, route, mode, returned, top_score, top_sources, keyword_candidates, vector_candidates, merged_candidates, text_only_results, vector_confirmed_results, weak_terms, created_at)
                 VALUES(?1, ?2, 'semantic', 'hybrid', 4, 1.0, '[]', 4, 1, 5, 2, 1, ?3, ?4)",
                params![
                    format!("q{idx}"),
                    "can does from with make money",
                    serde_json::to_string(&vec!["can", "does", "from", "with"]).unwrap(),
                    Utc::now().to_rfc3339(),
                ],
            )
            .unwrap();
        }
        let applied_again = apply_optimizer(&conn, 1).unwrap();
        assert!(
            applied_again.status.dynamic_stopwords.len() <= 3,
            "global stopword cap must hold across repeated unattended applies"
        );
    }

    #[test]
    fn rollback_consumes_the_degraded_event_batch() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_optimizer_schema(&conn).unwrap();
        let prior_state = OptimizerState {
            text_only_penalty: 0.91,
            dynamic_stopwords: vec!["how".to_string()],
            consecutive_adjustments: 3,
            last_applied_event_id: 10,
            last_vector_confirmed_ratio: Some(0.9),
            previous: Some(OptimizerSnapshot {
                text_only_penalty: 0.94,
                dynamic_stopwords: Vec::new(),
                last_vector_confirmed_ratio: Some(0.95),
            }),
            last_action: Some("apply".to_string()),
            last_applied_at: Some(Utc::now().to_rfc3339()),
            last_rollback_at: None,
        };
        save_state(&conn, &prior_state).unwrap();
        conn.execute(
            "INSERT INTO optimizer_events(id, query_id, query, route, mode, returned, top_score, top_sources, keyword_candidates, vector_candidates, merged_candidates, text_only_results, vector_confirmed_results, weak_terms, created_at)
             VALUES(11, 'q11', 'how make money', 'semantic', 'hybrid', 10, 1.0, '[]', 10, 1, 11, 5, 1, ?1, ?2)",
            params![
                serde_json::to_string(&vec!["how", "what", "why"]).unwrap(),
                Utc::now().to_rfc3339(),
            ],
        )
        .unwrap();

        let rolled_back = apply_optimizer(&conn, 1).unwrap();
        assert_eq!(rolled_back.status.last_action.as_deref(), Some("rollback"));
        assert_eq!(rolled_back.status.pending_events, 0);
        assert!((rolled_back.status.text_only_penalty - 0.94).abs() < 0.0001);
    }

    #[test]
    fn runtime_config_can_be_read_without_schema_migration() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE optimizer_state (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL);",
        )
        .unwrap();
        let state = OptimizerState {
            text_only_penalty: 0.82,
            dynamic_stopwords: vec![
                "how".to_string(),
                "what".to_string(),
                "why".to_string(),
                "with".to_string(),
                "how".to_string(),
            ],
            ..OptimizerState::default()
        };
        save_state(&conn, &state).unwrap();
        let config = load_runtime_config(&conn).unwrap();
        assert!((config.text_only_penalty - 0.82).abs() < f32::EPSILON);
        assert!(config.dynamic_stopwords.len() <= 3);
        assert_eq!(config.dynamic_stopwords, vec!["how", "what", "why"]);
    }
}
