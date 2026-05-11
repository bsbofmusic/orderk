
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
    pub embedding_provider: String,
    pub embedding_model: String,
    pub vector_backend: String,
    pub took_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub ok: bool,
    pub db: String,
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
    pub path_boost: f32,
    pub tag_boost: f32,
    pub recency_boost: f32,
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
    pub tags: Vec<String>,
    pub mtime: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub query: String,
    pub query_id: String,
    pub took_ms: u128,
    pub mode: String,
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
