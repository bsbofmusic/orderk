use crate::embedding::{EmbeddingProvider, MockEmbeddingProvider, SiliconFlowM3Provider};
use crate::index::{open_db, IndexStore};
use crate::models::*;
use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

pub fn init(
    db_path: &Path,
    embedding_dim: usize,
    embedding_model: &str,
    vector_backend: VectorBackend,
) -> Result<()> {
    let conn = open_db(db_path, embedding_dim, embedding_model, &vector_backend)?;
    let _ = IndexStore::status(&conn)?;
    Ok(())
}

pub fn index_vault(
    vault: &Path,
    db_path: &Path,
    provider: &dyn EmbeddingProvider,
    embedding_dim: usize,
    embedding_model: &str,
    vector_backend: VectorBackend,
) -> Result<IndexSummary> {
    let mut store = IndexStore::open(
        db_path,
        embedding_dim,
        embedding_model,
        &vector_backend,
        vault,
    )?;
    let summary = IndexStore::index_vault(
        &mut store.conn,
        vault,
        provider,
        embedding_dim,
        embedding_model,
        &vector_backend,
    )?;
    Ok(IndexSummary {
        db: db_path.to_string_lossy().to_string(),
        ..summary
    })
}

pub fn query(
    db_path: &Path,
    query: &str,
    limit: usize,
    provider: &dyn EmbeddingProvider,
    vector_backend: VectorBackend,
) -> Result<QueryResponse> {
    query_with_filter(db_path, query, limit, provider, vector_backend, None)
}

pub fn query_with_filter(
    db_path: &Path,
    query: &str,
    limit: usize,
    provider: &dyn EmbeddingProvider,
    vector_backend: VectorBackend,
    filter: Option<&str>,
) -> Result<QueryResponse> {
    let mut options = QueryOptions::new(limit);
    options.filter = filter
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    query_with_options(db_path, query, &options, provider, vector_backend)
}

pub fn query_with_options(
    db_path: &Path,
    query: &str,
    options: &QueryOptions,
    provider: &dyn EmbeddingProvider,
    vector_backend: VectorBackend,
) -> Result<QueryResponse> {
    let conn = open_writable_existing(db_path)?;
    IndexStore::query_with_options(&conn, query, options, provider, &vector_backend)
}

pub fn status(db_path: &Path) -> Result<StatusResponse> {
    let conn = open_existing(db_path)?;
    let mut status = IndexStore::status(&conn)?;
    status.db = db_path.to_string_lossy().to_string();
    Ok(status)
}

pub fn feedback(db_path: &Path, event: &FeedbackEvent) -> Result<FeedbackResponse> {
    let conn = open_writable_existing(db_path)?;
    IndexStore::feedback(&conn, event)
}

pub fn provider_from_env(dim: usize, model: Option<String>) -> Result<Box<dyn EmbeddingProvider>> {
    let key = std::env::var("HERMES_SILICONFLOW_API_KEY")
        .or_else(|_| std::env::var("SILICONFLOW_API_KEY"))
        .map_err(|_| anyhow::anyhow!("SiliconFlow API key is missing"))?;
    Ok(Box::new(SiliconFlowM3Provider::new(key, model, dim, None)))
}

pub fn provider_from_name(
    name: &str,
    dim: usize,
    model: Option<String>,
) -> Result<Box<dyn EmbeddingProvider>> {
    match name {
        "mock" => Ok(Box::new(MockEmbeddingProvider::new(dim))),
        "siliconflow" => {
            let key = std::env::var("HERMES_SILICONFLOW_API_KEY")
                .or_else(|_| std::env::var("SILICONFLOW_API_KEY"))
                .map_err(|_| anyhow::anyhow!("SiliconFlow API key is missing"))?;
            Ok(Box::new(SiliconFlowM3Provider::new(key, model, dim, None)))
        }
        other => Err(anyhow::anyhow!("unknown embedding provider: {}", other)),
    }
}

fn open_existing(db_path: &Path) -> Result<Connection> {
    crate::index::register_sqlite_vec();
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    Ok(conn)
}

fn open_writable_existing(db_path: &Path) -> Result<Connection> {
    crate::index::register_sqlite_vec();
    let conn = Connection::open(db_path)?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    crate::index::migrate_chunk_metadata_columns(&conn)?;
    Ok(conn)
}
