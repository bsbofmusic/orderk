use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct OrderkConfig {
    pub vault_path: PathBuf,
    pub db_path: PathBuf,
    pub embedding_dim: usize,
    pub embedding_model: String,
    pub vector_backend: VectorBackend,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VectorBackend {
    SqliteVec,
    Exact,
}

impl VectorBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SqliteVec => "sqlite_vec",
            Self::Exact => "exact",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannedFile {
    pub path: String,
    pub abs_path: PathBuf,
    pub mtime: i64,
    pub size: u64,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedDocument {
    pub path: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub wikilinks: Vec<String>,
    pub body: String,
    pub confidence: Option<String>,
    pub status: Option<String>,
    pub source_type: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub supersedes: Option<String>,
    pub superseded_by: Option<String>,
    pub updated: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: String,
    pub file_path: String,
    pub title: Option<String>,
    pub heading: Option<String>,
    pub line_start: usize,
    pub line_end: usize,
    pub text: String,
    pub hash: String,
    pub tags: Vec<String>,
    pub has_code: bool,
    pub has_link: bool,
    pub has_task_list: bool,
    pub has_incomplete_tasks: bool,
    pub confidence: Option<String>,
    pub status: Option<String>,
    pub source_type: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub supersedes: Option<String>,
    pub superseded_by: Option<String>,
    pub updated: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexOptions {
    #[serde(default = "default_chunk_max_chars")]
    pub chunk_max_chars: usize,
    #[serde(default)]
    pub chunk_overlap_chars: usize,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            chunk_max_chars: default_chunk_max_chars(),
            chunk_overlap_chars: 0,
        }
    }
}

impl IndexOptions {
    pub fn normalized(&self) -> Self {
        let chunk_max_chars = self.chunk_max_chars.max(200);
        Self {
            chunk_max_chars,
            chunk_overlap_chars: self.chunk_overlap_chars.min(chunk_max_chars / 2),
        }
    }

    pub fn strategy(&self) -> &'static str {
        if self.normalized().chunk_overlap_chars > 0 {
            "heading_overlap"
        } else {
            "heading"
        }
    }
}

pub fn default_chunk_max_chars() -> usize {
    1200
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSummary {
    pub ok: bool,
    pub vault: String,
    pub db: String,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub deleted: usize,
    pub files: usize,
    pub chunks: usize,
    pub embedded: usize,
    pub reused: usize,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub vector_backend: String,
    #[serde(default)]
    pub chunk_strategy: String,
    #[serde(default = "default_chunk_max_chars")]
    pub chunk_max_chars: usize,
    #[serde(default)]
    pub chunk_overlap_chars: usize,
    pub took_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Ready,
    NeedsIndex,
    Degraded,
    Unhealthy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    EDbOpenFailed,
    EDbCorrupt,
    ESchemaMissing,
    ENoEmbeddings,
    EProfileMismatch,
    EProviderDown,
    EVectorBackendMissing,
    EVaultUnreadable,
    ESmokeQueryFailed,
    EInvalidArgument,
    EUnknownProvider,
    EEmbeddingDimensionMismatch,
    EEmbeddingCountMismatch,
    EEmbeddingRequestFailed,
    EInternal,
}

impl ErrorCode {
    pub fn is_hard_failure(&self) -> bool {
        matches!(
            self,
            ErrorCode::EDbOpenFailed
                | ErrorCode::EDbCorrupt
                | ErrorCode::ESchemaMissing
                | ErrorCode::EProfileMismatch
                | ErrorCode::EProviderDown
                | ErrorCode::EVectorBackendMissing
                | ErrorCode::EVaultUnreadable
                | ErrorCode::ESmokeQueryFailed
                | ErrorCode::EInvalidArgument
                | ErrorCode::EUnknownProvider
                | ErrorCode::EEmbeddingDimensionMismatch
                | ErrorCode::EEmbeddingCountMismatch
                | ErrorCode::EEmbeddingRequestFailed
                | ErrorCode::EInternal
        )
    }
}

impl HealthState {
    pub fn from_error_codes(codes: &[ErrorCode]) -> Self {
        if codes.is_empty() {
            return HealthState::Ready;
        }
        if codes.len() == 1 && codes[0] == ErrorCode::ENoEmbeddings {
            return HealthState::NeedsIndex;
        }
        if codes.iter().any(ErrorCode::is_hard_failure) {
            HealthState::Unhealthy
        } else {
            HealthState::Degraded
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub component: String,
    pub ok: bool,
    pub error_code: Option<ErrorCode>,
    pub message: String,
    pub remediation: Option<String>,
    pub details: serde_json::Value,
}

impl HealthCheck {
    pub fn ok(
        component: impl Into<String>,
        message: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        Self {
            component: component.into(),
            ok: true,
            error_code: None,
            message: message.into(),
            remediation: None,
            details,
        }
    }

    pub fn fail(
        component: impl Into<String>,
        error_code: ErrorCode,
        message: impl Into<String>,
        remediation: impl Into<Option<String>>,
        details: serde_json::Value,
    ) -> Self {
        Self {
            component: component.into(),
            ok: false,
            error_code: Some(error_code),
            message: message.into(),
            remediation: remediation.into(),
            details,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub schema_version: String,
    pub ok: bool,
    pub state: HealthState,
    pub db: String,
    pub vault: Option<String>,
    pub checks: Vec<HealthCheck>,
    pub error_codes: Vec<ErrorCode>,
    pub status: Option<StatusResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub ok: bool,
    pub schema_version: String,
    pub db: String,
    pub health_state: HealthState,
    pub error_codes: Vec<ErrorCode>,
    pub checks: Vec<HealthCheck>,
    pub notes: usize,
    pub chunks: usize,
    pub embeddings: usize,
    pub fts_enabled: bool,
    pub vector_enabled: bool,
    pub vector_backend: String,
    pub vec_version: Option<String>,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub embedding_dim: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScoreBreakdown {
    pub keyword: f32,
    pub vector: f32,
    pub fusion: f32,
    pub path_boost: f32,
    pub tag_boost: f32,
    pub route_boost: f32,
    pub recency_boost: f32,
    pub metadata_boost: f32,
    #[serde(default)]
    pub link_boost: f32,
    #[serde(default)]
    pub optimizer_adjustment: f32,
    #[serde(default)]
    pub freshness_boost: f32,
    #[serde(default)]
    pub confidence_boost: f32,
    #[serde(default)]
    pub status_boost: f32,
    #[serde(default)]
    pub evidence_count_boost: f32,
    #[serde(default)]
    pub reranker_boost: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessMode {
    Off,
    #[default]
    Balanced,
    Recent,
    Oldest,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ValidityEvidence {
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_days: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QualitySummary {
    pub schema_version: String,
    pub freshness_boost: f32,
    pub confidence_boost: f32,
    pub status_boost: f32,
    pub evidence_count_boost: f32,
    pub total_boost: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvidenceSummary {
    pub schema_version: String,
    pub validity_state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_days: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    pub evidence_count: usize,
    pub sources: Vec<String>,
    #[serde(default)]
    pub evidence_uri: String,
    #[serde(default)]
    pub open_uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LinkEvidence {
    pub outgoing: Vec<OutgoingLinkEvidence>,
    pub backlinks: Vec<BacklinkEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingLinkEvidence {
    pub target: String,
    pub normalized_target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklinkEvidence {
    pub source_path: String,
    pub source_title: Option<String>,
    pub target: String,
    pub normalized_target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchResultEvidence {
    pub sources: Vec<String>,
    #[serde(default)]
    pub evidence_count: usize,
    #[serde(default)]
    pub retrieval_depth: usize,
    pub keyword_rank: Option<usize>,
    pub vector_rank: Option<usize>,
    pub route: Option<String>,
    pub route_score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<LinkEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchContextChunk {
    pub relation: String,
    pub chunk_id: String,
    pub path: String,
    pub heading: Option<String>,
    pub line_start: usize,
    pub line_end: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryOptions {
    pub limit: usize,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub min_score: Option<f32>,
    #[serde(default)]
    pub context_chunks: usize,
    #[serde(default)]
    pub include_links: bool,
    #[serde(default = "default_rerank")]
    pub rerank: bool,
    #[serde(default)]
    pub expand_links: usize,
    #[serde(default)]
    pub retrieval_depth: usize,
    #[serde(default)]
    pub explain: bool,
    #[serde(default)]
    pub freshness: FreshnessMode,
    #[serde(default)]
    pub as_of: Option<String>,
    #[serde(default)]
    pub include_stale: bool,
    #[serde(default)]
    pub query_expansion: bool,
    #[serde(default)]
    pub external_reranker: bool,
}

fn default_rerank() -> bool {
    true
}

impl QueryOptions {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            filter: None,
            min_score: None,
            context_chunks: 0,
            include_links: false,
            rerank: default_rerank(),
            expand_links: 0,
            retrieval_depth: 0,
            explain: false,
            freshness: FreshnessMode::default(),
            as_of: None,
            include_stale: false,
            query_expansion: false,
            external_reranker: false,
        }
    }

    pub fn effective_retrieval_depth(&self) -> Result<usize, String> {
        if self.expand_links > 1 {
            return Err("--expand-links currently supports 0 or 1".to_string());
        }
        if self.retrieval_depth > 1 {
            return Err("--retrieval-depth currently supports 0 or 1".to_string());
        }
        Ok(self.expand_links.max(self.retrieval_depth))
    }
}

#[cfg(test)]
mod model_contract_tests {
    use super::*;

    #[test]
    fn query_options_default_keeps_explain_trace_off() {
        let options = QueryOptions::new(3);
        assert!(!options.explain);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueryTimings {
    pub keyword_ms: u128,
    pub vector_ms: u128,
    pub route_ms: u128,
    pub merge_ms: u128,
    pub link_expansion_ms: u128,
    pub enrich_ms: u128,
    pub total_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueryRoutingEvidence {
    pub strategy: String,
    pub route: String,
    pub routes_attempted: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_profile_fingerprint: Option<String>,
    pub filter: Option<String>,
    pub filter_mode: Option<String>,
    pub filtered_candidates: Option<usize>,
    pub min_score: Option<f32>,
    pub threshold_filtered: Option<usize>,
    pub context_chunks: usize,
    pub include_links: bool,
    pub expand_links: usize,
    #[serde(default)]
    pub retrieval_depth: usize,
    #[serde(default)]
    pub query_expansion: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub query_expansion_terms: Vec<String>,
    #[serde(default)]
    pub external_reranker: bool,
    pub keyword_candidates: usize,
    pub vector_candidates: usize,
    pub route_candidates: usize,
    pub link_candidates: usize,
    pub merged_candidates: usize,
    pub returned: usize,
    pub timings: QueryTimings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryExplainStage {
    pub name: String,
    pub candidates: usize,
    pub took_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryExplainResult {
    pub rank: usize,
    pub chunk_id: String,
    pub path: String,
    pub score: f32,
    pub sources: Vec<String>,
    pub keyword_rank: Option<usize>,
    pub vector_rank: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryExplainTrace {
    pub schema_version: String,
    pub route: String,
    pub strategy: String,
    pub vector_backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_profile_fingerprint: Option<String>,
    pub limit: usize,
    pub returned: usize,
    pub filter: Option<String>,
    pub min_score: Option<f32>,
    pub retrieval_depth: usize,
    pub timings: QueryTimings,
    pub stages: Vec<QueryExplainStage>,
    pub result_ranks: Vec<QueryExplainResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub chunk_id: String,
    pub file_path: String,
    pub path: String,
    pub title: Option<String>,
    pub heading: Option<String>,
    pub line_start: usize,
    pub line_end: usize,
    #[serde(default)]
    pub evidence_uri: String,
    #[serde(default)]
    pub open_uri: String,
    pub snippet: String,
    pub score: f32,
    pub score_breakdown: ScoreBreakdown,
    pub evidence: SearchResultEvidence,
    #[serde(default)]
    pub quality: QualitySummary,
    #[serde(default)]
    pub evidence_summary: EvidenceSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_chunks: Vec<SearchContextChunk>,
    pub tags: Vec<String>,
    pub confidence: Option<String>,
    pub status: Option<String>,
    pub source_type: Option<String>,
    #[serde(default)]
    pub validity: ValidityEvidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    pub mtime: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub query: String,
    pub query_id: String,
    pub took_ms: u128,
    pub mode: String,
    pub route: String,
    pub routing: QueryRoutingEvidence,
    pub vector_backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<QueryExplainTrace>,
    pub results: Vec<SearchResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimizer: Option<OptimizerStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptimizerRuntimeConfig {
    pub text_only_penalty: f32,
    pub dynamic_stopwords: Vec<String>,
}

impl Default for OptimizerRuntimeConfig {
    fn default() -> Self {
        Self {
            text_only_penalty: 1.0,
            dynamic_stopwords: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OptimizerStatus {
    pub schema_version: String,
    pub enabled: bool,
    pub message: String,
    pub total_events: usize,
    pub pending_events: usize,
    pub text_only_penalty: f64,
    pub dynamic_stopwords: Vec<String>,
    pub consecutive_adjustments: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_applied_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_rollback_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OptimizerMetrics {
    pub events: usize,
    pub returned_results: usize,
    pub text_only_results: usize,
    pub vector_confirmed_results: usize,
    pub text_only_ratio: f64,
    pub vector_confirmed_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OptimizeProposal {
    pub schema_version: String,
    pub eligible: bool,
    pub reason: String,
    pub stopwords_to_add: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_only_penalty_from: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_only_penalty_to: Option<f64>,
    pub latest_event_id: i64,
    pub metrics: OptimizerMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OptimizeResponse {
    pub schema_version: String,
    pub ok: bool,
    pub mode: String,
    pub status: OptimizerStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<OptimizeProposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchIndexEntry {
    pub chunk_id: String,
    pub title: Option<String>,
    pub score: f32,
    pub path: String,
    pub heading: Option<String>,
    pub line_start: usize,
    pub line_end: usize,
    #[serde(default)]
    pub evidence_uri: String,
    #[serde(default)]
    pub open_uri: String,
    #[serde(default)]
    pub validity: ValidityEvidence,
    #[serde(default)]
    pub quality: QualitySummary,
    #[serde(default)]
    pub evidence_summary: EvidenceSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchIndexResponse {
    pub schema_version: String,
    pub query: String,
    pub query_id: String,
    pub took_ms: u128,
    pub view: String,
    pub mode: String,
    pub route: String,
    pub routing: QueryRoutingEvidence,
    pub vector_backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<QueryExplainTrace>,
    pub results: Vec<SearchIndexEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimizer: Option<OptimizerStatus>,
}

impl From<QueryResponse> for SearchIndexResponse {
    fn from(response: QueryResponse) -> Self {
        let results = response
            .results
            .into_iter()
            .map(|result| SearchIndexEntry {
                chunk_id: result.chunk_id,
                title: result
                    .title
                    .or_else(|| result.heading.clone())
                    .or(Some(result.path.clone())),
                score: result.score,
                path: result.path,
                heading: result.heading,
                line_start: result.line_start,
                line_end: result.line_end,
                evidence_uri: result.evidence_uri,
                open_uri: result.open_uri,
                validity: result.validity,
                quality: result.quality,
                evidence_summary: result.evidence_summary,
            })
            .collect();
        let explain = response.explain;
        let optimizer = response.optimizer;
        Self {
            schema_version: "orderk.search_index.v1".to_string(),
            query: response.query,
            query_id: response.query_id,
            took_ms: response.took_ms,
            view: "index".to_string(),
            mode: response.mode,
            route: response.route,
            routing: response.routing,
            vector_backend: response.vector_backend,
            explain,
            optimizer,
            results,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChunkGetDetail {
    Summary,
    Full,
}

impl ChunkGetDetail {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkGetOptions {
    pub chunk_ids: Vec<String>,
    #[serde(default = "default_chunk_get_detail")]
    pub detail: ChunkGetDetail,
    #[serde(default)]
    pub context_chunks: usize,
}

fn default_chunk_get_detail() -> ChunkGetDetail {
    ChunkGetDetail::Full
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkGetResult {
    pub chunk_id: String,
    pub path: String,
    pub title: Option<String>,
    pub heading: Option<String>,
    pub line_start: usize,
    pub line_end: usize,
    #[serde(default)]
    pub evidence_uri: String,
    #[serde(default)]
    pub open_uri: String,
    pub text: String,
    pub tags: Vec<String>,
    pub confidence: Option<String>,
    pub status: Option<String>,
    pub source_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    pub mtime: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_chunks: Vec<SearchContextChunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkGetResponse {
    pub schema_version: String,
    pub total: usize,
    pub detail: ChunkGetDetail,
    pub results: Vec<ChunkGetResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackEvent {
    pub event: String,
    pub query_id: Option<String>,
    pub chunk_id: Option<String>,
    pub query: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackResponse {
    pub ok: bool,
    pub event_id: i64,
}
