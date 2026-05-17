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
    pub keyword_candidates: usize,
    pub vector_candidates: usize,
    pub route_candidates: usize,
    pub link_candidates: usize,
    pub merged_candidates: usize,
    pub returned: usize,
    pub timings: QueryTimings,
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
    pub snippet: String,
    pub score: f32,
    pub score_breakdown: ScoreBreakdown,
    pub evidence: SearchResultEvidence,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_chunks: Vec<SearchContextChunk>,
    pub tags: Vec<String>,
    pub confidence: Option<String>,
    pub status: Option<String>,
    pub source_type: Option<String>,
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
    pub results: Vec<SearchResult>,
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
