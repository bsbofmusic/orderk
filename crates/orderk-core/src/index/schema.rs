//! Database bootstrap: connection setup, schema DDL, migrations, and profile guards.
//!
//! Owns `open_db`/`init_schema` (the crate's public DB entry points), sqlite-vec
//! extension registration, chunk-metadata column migrations, and the
//! embedding/schema profile consistency checks. Extracted from `index.rs`.

use super::links::extract_wikilinks_from_text;
use super::settings::{
    bool_to_i64, indexed_embedding_count, load_settings_map, require_setting, upsert_setting,
    vec_table_exists,
};
use crate::chunker::{has_code, has_incomplete_tasks, has_link, has_task_list};
use crate::embedding::EmbeddingProvider;
use crate::models::*;
use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sqlite_vec::sqlite3_vec_init;
use std::fs;
use std::path::Path;
use std::sync::Once;

static SQLITE_VEC_REGISTER: Once = Once::new();

pub(crate) fn register_sqlite_vec() {
    SQLITE_VEC_REGISTER.call_once(|| unsafe {
        type SqliteAutoExtension = unsafe extern "C" fn(
            *mut rusqlite::ffi::sqlite3,
            *mut *const i8,
            *const rusqlite::ffi::sqlite3_api_routines,
        ) -> i32;
        let extension =
            std::mem::transmute::<*const (), SqliteAutoExtension>(sqlite3_vec_init as *const ());
        rusqlite::ffi::sqlite3_auto_extension(Some(extension));
    });
}

pub fn open_db(
    path: &Path,
    embedding_dim: usize,
    embedding_model: &str,
    vector_backend: &VectorBackend,
) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    register_sqlite_vec();
    let conn = Connection::open(path).with_context(|| format!("open db {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
    )?;
    init_schema(&conn, embedding_dim, embedding_model, vector_backend)?;
    Ok(conn)
}

pub fn init_schema(
    conn: &Connection,
    embedding_dim: usize,
    embedding_model: &str,
    vector_backend: &VectorBackend,
) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS files (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            path TEXT NOT NULL UNIQUE,
            mtime INTEGER NOT NULL,
            size INTEGER NOT NULL,
            hash TEXT NOT NULL,
            indexed_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS chunks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            chunk_id TEXT NOT NULL UNIQUE,
            file_path TEXT NOT NULL,
            file_hash TEXT NOT NULL,
            title TEXT,
            heading TEXT,
            line_start INTEGER NOT NULL,
            line_end INTEGER NOT NULL,
            text TEXT NOT NULL,
            tags_json TEXT NOT NULL,
            links_json TEXT NOT NULL DEFAULT '[]',
            has_code INTEGER NOT NULL DEFAULT 0,
            has_link INTEGER NOT NULL DEFAULT 0,
            has_task_list INTEGER NOT NULL DEFAULT 0,
            has_incomplete_tasks INTEGER NOT NULL DEFAULT 0,
            confidence TEXT,
            status TEXT,
            source_type TEXT,
            valid_from TEXT,
            valid_until TEXT,
            supersedes TEXT,
            superseded_by TEXT,
            updated TEXT,
            chunk_hash TEXT NOT NULL,
            mtime INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS chunk_embeddings (
            chunk_id TEXT PRIMARY KEY,
            model TEXT NOT NULL,
            dim INTEGER NOT NULL,
            embedding BLOB NOT NULL,
            vector_hash TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS feedback_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event TEXT NOT NULL,
            query_id TEXT,
            chunk_id TEXT,
            query TEXT,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS fts_chunks USING fts5(
            chunk_id UNINDEXED,
            file_path UNINDEXED,
            title,
            heading,
            text,
            tags,
            tokenize = 'unicode61 remove_diacritics 2'
        );
        "#,
    )?;

    migrate_chunk_metadata_columns(conn)?;
    ensure_schema_profile(conn, embedding_dim, embedding_model, vector_backend)?;
    let vec_sql = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_chunks USING vec0(embedding float[{}])",
        embedding_dim
    );
    conn.execute_batch(&vec_sql)?;
    upsert_setting(conn, "embedding_dim", &embedding_dim.to_string())?;
    upsert_setting(conn, "embedding_model", embedding_model)?;
    upsert_setting(conn, "vector_backend", vector_backend.as_str())?;
    upsert_setting(conn, "vector_backend_mode", vector_backend.as_str())?;
    upsert_setting(conn, "schema_version", "5")?;
    Ok(())
}

pub(crate) fn migrate_chunk_metadata_columns(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "chunks")? {
        return Ok(());
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )?;
    let schema_version = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let migrations = [
        (
            "links_json",
            "ALTER TABLE chunks ADD COLUMN links_json TEXT NOT NULL DEFAULT '[]'",
        ),
        (
            "has_code",
            "ALTER TABLE chunks ADD COLUMN has_code INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "has_link",
            "ALTER TABLE chunks ADD COLUMN has_link INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "has_task_list",
            "ALTER TABLE chunks ADD COLUMN has_task_list INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "has_incomplete_tasks",
            "ALTER TABLE chunks ADD COLUMN has_incomplete_tasks INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "confidence",
            "ALTER TABLE chunks ADD COLUMN confidence TEXT",
        ),
        ("status", "ALTER TABLE chunks ADD COLUMN status TEXT"),
        (
            "source_type",
            "ALTER TABLE chunks ADD COLUMN source_type TEXT",
        ),
        (
            "valid_from",
            "ALTER TABLE chunks ADD COLUMN valid_from TEXT",
        ),
        (
            "valid_until",
            "ALTER TABLE chunks ADD COLUMN valid_until TEXT",
        ),
        (
            "supersedes",
            "ALTER TABLE chunks ADD COLUMN supersedes TEXT",
        ),
        (
            "superseded_by",
            "ALTER TABLE chunks ADD COLUMN superseded_by TEXT",
        ),
        ("updated", "ALTER TABLE chunks ADD COLUMN updated TEXT"),
    ];
    let mut added_any = false;
    for (column, sql) in migrations {
        if !chunk_column_exists(conn, column)? {
            conn.execute_batch(sql)?;
            added_any = true;
        }
    }
    if added_any || schema_version.as_deref() != Some("5") {
        backfill_chunk_metadata(conn)?;
    }
    upsert_setting(conn, "schema_version", "5")?;
    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![table],
        |row| row.get::<_, usize>(0),
    )? > 0)
}

pub(crate) fn chunk_column_exists(conn: &Connection, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare("PRAGMA table_info(chunks)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn backfill_chunk_metadata(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("SELECT id, text FROM chunks")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    drop(stmt);
    for (id, text) in records {
        let links_json = serde_json::to_string(&extract_wikilinks_from_text(&text))?;
        conn.execute(
            "UPDATE chunks SET links_json = ?1, has_code = ?2, has_link = ?3, has_task_list = ?4, has_incomplete_tasks = ?5 WHERE id = ?6",
            params![
                links_json,
                bool_to_i64(has_code(&text)),
                bool_to_i64(has_link(&text)),
                bool_to_i64(has_task_list(&text)),
                bool_to_i64(has_incomplete_tasks(&text)),
                id,
            ],
        )?;
    }
    Ok(())
}

fn ensure_schema_profile(
    conn: &Connection,
    embedding_dim: usize,
    embedding_model: &str,
    vector_backend: &VectorBackend,
) -> Result<()> {
    let settings = load_settings_map(conn)?;
    let embeddings = indexed_embedding_count(conn)?;
    if embeddings > 0 {
        require_setting(&settings, "embedding_dim", &embedding_dim.to_string())?;
        require_setting(&settings, "embedding_model", embedding_model)?;
        require_setting(&settings, "vector_backend", vector_backend.as_str())?;
        return Ok(());
    }

    if vec_table_exists(conn) {
        let needs_rebuild = match settings.get("embedding_dim") {
            Some(existing_dim) => existing_dim != &embedding_dim.to_string(),
            None => true,
        };
        if needs_rebuild {
            conn.execute_batch("DROP TABLE IF EXISTS vec_chunks;")?;
        }
    }
    Ok(())
}

pub(crate) fn ensure_runtime_profile<P: EmbeddingProvider + ?Sized>(
    conn: &Connection,
    provider: &P,
    embedding_dim: usize,
    embedding_model: &str,
    vector_backend: &VectorBackend,
) -> Result<()> {
    let embeddings = indexed_embedding_count(conn)?;
    if embeddings == 0 {
        return Ok(());
    }
    let settings = load_settings_map(conn)?;
    require_setting(&settings, "embedding_provider", provider.provider_id())?;
    require_setting(&settings, "embedding_dim", &embedding_dim.to_string())?;
    require_setting(&settings, "embedding_model", embedding_model)?;
    require_setting(&settings, "vector_backend", vector_backend.as_str())?;
    Ok(())
}

pub(crate) fn ensure_has_embeddings(conn: &Connection) -> Result<()> {
    if indexed_embedding_count(conn)? == 0 {
        return Err(anyhow!(
            "orderk database has no embeddings yet; run `orderk index` before search"
        ));
    }
    Ok(())
}
