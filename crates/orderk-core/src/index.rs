use crate::chunker::{
    chunk_document_with_options, has_code, has_incomplete_tasks, has_link, has_task_list,
    ChunkingOptions,
};
use crate::embedding::{vector_hash, EmbeddingProvider};
use crate::filter::{compile_filter, FilterSql};
use crate::markdown::parse_markdown;
use crate::models::*;
use crate::optimizer;
use crate::scanner::scan_vault;
use anyhow::{anyhow, Context, Result};
use chrono::{NaiveDate, TimeZone, Utc};
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use sqlite_vec::sqlite3_vec_init;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Once;
use std::time::Instant;

static SQLITE_VEC_REGISTER: Once = Once::new();
const LINK_EXPANSION_BOOST: f32 = 0.03;
const LINK_EXPANSION_SEED_LIMIT: usize = 10;

mod scoring;
use scoring::*;
mod uri;
use uri::*;
mod query_plan;
use query_plan::*;

fn build_query_explain_trace(
    routing: &QueryRoutingEvidence,
    results: &[SearchResult],
    vector_backend: &str,
    limit: usize,
) -> QueryExplainTrace {
    QueryExplainTrace {
        schema_version: "orderk.explain_trace.v1".to_string(),
        route: routing.route.clone(),
        strategy: routing.strategy.clone(),
        vector_backend: vector_backend.to_string(),
        limit,
        returned: results.len(),
        filter: routing.filter.clone(),
        min_score: routing.min_score,
        retrieval_depth: routing.retrieval_depth,
        timings: routing.timings.clone(),
        stages: vec![
            QueryExplainStage {
                name: "keyword".to_string(),
                candidates: routing.keyword_candidates,
                took_ms: routing.timings.keyword_ms,
            },
            QueryExplainStage {
                name: "vector".to_string(),
                candidates: routing.vector_candidates,
                took_ms: routing.timings.vector_ms,
            },
            QueryExplainStage {
                name: "route".to_string(),
                candidates: routing.route_candidates,
                took_ms: routing.timings.route_ms,
            },
            QueryExplainStage {
                name: "link_expansion".to_string(),
                candidates: routing.link_candidates,
                took_ms: routing.timings.link_expansion_ms,
            },
            QueryExplainStage {
                name: "merge".to_string(),
                candidates: routing.merged_candidates,
                took_ms: routing.timings.merge_ms,
            },
            QueryExplainStage {
                name: "enrich".to_string(),
                candidates: results.len(),
                took_ms: routing.timings.enrich_ms,
            },
        ],
        result_ranks: results
            .iter()
            .enumerate()
            .map(|(idx, result)| QueryExplainResult {
                rank: idx + 1,
                chunk_id: result.chunk_id.clone(),
                path: result.path.clone(),
                score: result.score,
                sources: result.evidence.sources.clone(),
                keyword_rank: result.evidence.keyword_rank,
                vector_rank: result.evidence.vector_rank,
            })
            .collect(),
    }
}

#[derive(Debug, Clone)]
struct RouteHit {
    score: f32,
    reasons: Vec<String>,
}

#[derive(Debug, Clone)]
struct EmbeddingRecord {
    blob: Vec<u8>,
    vector_hash: String,
}

#[derive(Debug, Clone)]
struct ReindexFileSummary {
    chunks: usize,
    embedded: usize,
    reused: usize,
}

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

fn chunk_column_exists(conn: &Connection, column: &str) -> Result<bool> {
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

fn bool_to_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn extract_wikilinks_from_text(text: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("]]") else {
            break;
        };
        let raw = after_start[..end].trim();
        if !raw.is_empty() {
            links.push(raw.to_string());
        }
        rest = &after_start[end + 2..];
    }
    links.sort();
    links.dedup();
    links
}

fn normalize_wikilink_target(target: &str) -> String {
    let target = target
        .split('|')
        .next()
        .unwrap_or(target)
        .split('#')
        .next()
        .unwrap_or(target)
        .trim();
    let without_md = target.strip_suffix(".md").unwrap_or(target);
    without_md
        .rsplit('/')
        .next()
        .unwrap_or(without_md)
        .trim()
        .to_lowercase()
}

fn path_stem(path: &str) -> String {
    let filename = path.rsplit('/').next().unwrap_or(path);
    filename
        .strip_suffix(".md")
        .unwrap_or(filename)
        .to_lowercase()
}

fn title_key(title: Option<&str>) -> Option<String> {
    title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(|title| title.to_lowercase())
}

fn link_points_to(link: &str, path: &str, title: Option<&str>) -> bool {
    let normalized = normalize_wikilink_target(link);
    normalized == path_stem(path) || title_key(title).as_deref() == Some(normalized.as_str())
}

fn upsert_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn setting_value<'a>(settings: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    settings.get(key).map(String::as_str)
}

fn chunk_profile_matches(settings: &HashMap<String, String>, options: &IndexOptions) -> bool {
    let options = options.normalized();
    let existing_max = setting_value(settings, "chunk_max_chars")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_else(default_chunk_max_chars);
    let existing_overlap = setting_value(settings, "chunk_overlap_chars")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let existing_strategy =
        setting_value(settings, "chunk_strategy").unwrap_or(if existing_overlap > 0 {
            "heading_overlap"
        } else {
            "heading"
        });
    existing_max == options.chunk_max_chars
        && existing_overlap == options.chunk_overlap_chars
        && existing_strategy == options.strategy()
}

fn load_settings_map(conn: &Connection) -> Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut map = HashMap::new();
    for row in rows {
        let (key, value) = row?;
        map.insert(key, value);
    }
    Ok(map)
}

fn indexed_embedding_count(conn: &Connection) -> Result<usize> {
    Ok(conn.query_row("SELECT COUNT(*) FROM chunk_embeddings", [], |r| r.get(0))?)
}

fn vec_table_exists(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'vec_chunks'",
        [],
        |r| r.get::<_, usize>(0),
    )
    .unwrap_or(0)
        > 0
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

fn ensure_runtime_profile<P: EmbeddingProvider + ?Sized>(
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

fn ensure_has_embeddings(conn: &Connection) -> Result<()> {
    if indexed_embedding_count(conn)? == 0 {
        return Err(anyhow!(
            "orderk database has no embeddings yet; run `orderk index` before search"
        ));
    }
    Ok(())
}

fn require_setting(settings: &HashMap<String, String>, key: &str, expected: &str) -> Result<()> {
    match settings.get(key) {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(anyhow!(
            "orderk index profile mismatch for {}: existing `{}`, requested `{}`. Rebuild the index DB or use matching provider/model/dim/backend flags.",
            key,
            actual,
            expected
        )),
        None => Err(anyhow!(
            "orderk index profile missing `{}`. Rebuild the index DB before using this store.",
            key
        )),
    }
}

pub struct IndexStore {
    pub conn: Connection,
    pub vault_path: String,
    pub db_path: String,
}

impl IndexStore {
    pub fn open(
        db_path: &Path,
        embedding_dim: usize,
        embedding_model: &str,
        vector_backend: &VectorBackend,
        vault_path: &Path,
    ) -> Result<Self> {
        Ok(Self {
            conn: open_db(db_path, embedding_dim, embedding_model, vector_backend)?,
            vault_path: vault_path.to_string_lossy().to_string(),
            db_path: db_path.to_string_lossy().to_string(),
        })
    }

    pub fn load_settings(conn: &Connection) -> Result<HashMap<String, String>> {
        load_settings_map(conn)
    }

    pub fn status(conn: &Connection) -> Result<StatusResponse> {
        let settings = Self::load_settings(conn)?;
        let notes: usize = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        let chunks: usize = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
        let embeddings: usize =
            conn.query_row("SELECT COUNT(*) FROM chunk_embeddings", [], |r| r.get(0))?;
        let vec_version = conn
            .query_row("SELECT vec_version()", [], |r| r.get::<_, String>(0))
            .optional()
            .unwrap_or(None);
        let vec_tables: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'vec_chunks'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let embedding_dim = settings
            .get("embedding_dim")
            .and_then(|v| v.parse().ok())
            .unwrap_or_default();
        let vector_backend = settings
            .get("vector_backend")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        let vector_enabled = vec_version.is_some() && vec_tables == 1;
        let mut checks = vec![HealthCheck::ok(
            "db",
            "SQLite schema is reachable",
            serde_json::json!({"settings": settings.len(), "tables": {"vec_chunks": vec_tables}}),
        )];
        if embeddings == 0 {
            checks.push(HealthCheck::fail(
                "embeddings",
                ErrorCode::ENoEmbeddings,
                "database has no embeddings yet",
                Some(
                    "run `orderk index` with the same provider/model/dim/backend profile"
                        .to_string(),
                ),
                serde_json::json!({"chunks": chunks, "embeddings": embeddings}),
            ));
        }
        if vector_backend == VectorBackend::SqliteVec.as_str() && !vector_enabled {
            checks.push(HealthCheck::fail(
                "vector_backend",
                ErrorCode::EVectorBackendMissing,
                "sqlite-vec backend is configured but not available",
                Some("re-run `orderk init`/`orderk index` in an environment with sqlite-vec available".to_string()),
                serde_json::json!({"vec_version": vec_version, "vec_tables": vec_tables}),
            ));
        }
        let error_codes: Vec<ErrorCode> =
            checks.iter().filter_map(|c| c.error_code.clone()).collect();
        let health_state = HealthState::from_error_codes(&error_codes);
        Ok(StatusResponse {
            ok: health_state == HealthState::Ready,
            schema_version: "orderk.status.v1".to_string(),
            db: String::new(),
            health_state,
            error_codes,
            checks,
            notes,
            chunks,
            embeddings,
            fts_enabled: true,
            vector_enabled,
            vector_backend,
            vec_version,
            embedding_provider: settings
                .get("embedding_provider")
                .cloned()
                .unwrap_or_else(|| "unknown".to_string()),
            embedding_model: settings.get("embedding_model").cloned().unwrap_or_default(),
            embedding_dim,
        })
    }

    pub fn index_vault<P: EmbeddingProvider + ?Sized>(
        conn: &mut Connection,
        vault: &Path,
        provider: &P,
        embedding_dim: usize,
        embedding_model: &str,
        vector_backend: &VectorBackend,
    ) -> Result<IndexSummary> {
        Self::index_vault_with_options(
            conn,
            vault,
            provider,
            embedding_dim,
            embedding_model,
            vector_backend,
            &IndexOptions::default(),
        )
    }

    pub fn index_vault_with_options<P: EmbeddingProvider + ?Sized>(
        conn: &mut Connection,
        vault: &Path,
        provider: &P,
        embedding_dim: usize,
        embedding_model: &str,
        vector_backend: &VectorBackend,
        options: &IndexOptions,
    ) -> Result<IndexSummary> {
        let started = Instant::now();
        init_schema(conn, embedding_dim, embedding_model, vector_backend)?;
        provider.health()?;
        ensure_runtime_profile(
            conn,
            provider,
            embedding_dim,
            embedding_model,
            vector_backend,
        )?;
        conn.execute(
            "INSERT INTO settings(key, value) VALUES('embedding_provider', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![provider.provider_id()],
        )?;

        let scanned = scan_vault(vault)?;
        let chunk_options = options.normalized();
        let chunk_strategy = chunk_options.strategy().to_string();
        let mut seen_paths = HashSet::new();
        let mut existing: HashMap<String, (i64, String)> = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT path, id, hash FROM files")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (path, id, hash) = row?;
                existing.insert(path, (id, hash));
            }
        }

        let settings = load_settings_map(conn)?;
        let profile_changed = !chunk_profile_matches(&settings, &chunk_options);
        let mut added = 0usize;
        let mut updated = 0usize;
        let mut unchanged = 0usize;
        let mut deleted = 0usize;
        let mut embedded = 0usize;
        let mut reused = 0usize;
        let mut total_chunks = 0usize;

        for file in &scanned {
            seen_paths.insert(file.path.clone());
            let state = existing.get(&file.path).cloned();
            match state {
                None => added += 1,
                Some((_, ref hash)) if hash != &file.hash || profile_changed => updated += 1,
                Some(_) => unchanged += 1,
            }
        }

        let to_delete: Vec<String> = existing
            .keys()
            .filter(|path| !seen_paths.contains(*path))
            .cloned()
            .collect();
        for path in &to_delete {
            delete_file(conn, path)?;
            deleted += 1;
        }

        for file in &scanned {
            let needs_reindex = match existing.get(&file.path) {
                None => true,
                Some((_, hash)) => hash != &file.hash,
            };
            if !needs_reindex && !profile_changed {
                continue;
            }
            let file_summary = reindex_file_with_options(
                conn,
                file,
                provider,
                embedding_dim,
                embedding_model,
                vector_backend,
                &chunk_options,
            )?;
            total_chunks += file_summary.chunks;
            embedded += file_summary.embedded;
            reused += file_summary.reused;
        }

        upsert_setting(conn, "embedding_provider", provider.provider_id())?;
        upsert_setting(conn, "embedding_model", embedding_model)?;
        upsert_setting(conn, "embedding_dim", &embedding_dim.to_string())?;
        upsert_setting(conn, "vector_backend", vector_backend.as_str())?;
        upsert_setting(conn, "chunk_strategy", &chunk_strategy)?;
        upsert_setting(
            conn,
            "chunk_max_chars",
            &chunk_options.chunk_max_chars.to_string(),
        )?;
        upsert_setting(
            conn,
            "chunk_overlap_chars",
            &chunk_options.chunk_overlap_chars.to_string(),
        )?;

        Ok(IndexSummary {
            ok: true,
            vault: vault.to_string_lossy().to_string(),
            db: String::new(),
            added,
            updated,
            unchanged,
            deleted,
            files: scanned.len(),
            chunks: total_chunks,
            embedded,
            reused,
            embedding_provider: provider.provider_id().to_string(),
            embedding_model: provider.model_id().to_string(),
            vector_backend: vector_backend.as_str().to_string(),
            chunk_strategy,
            chunk_max_chars: chunk_options.chunk_max_chars,
            chunk_overlap_chars: chunk_options.chunk_overlap_chars,
            took_ms: started.elapsed().as_millis(),
        })
    }

    pub fn query<P: EmbeddingProvider + ?Sized>(
        conn: &Connection,
        query: &str,
        limit: usize,
        provider: &P,
        vector_backend: &VectorBackend,
    ) -> Result<QueryResponse> {
        Self::query_with_filter(conn, query, limit, provider, vector_backend, None)
    }

    pub fn query_with_filter<P: EmbeddingProvider + ?Sized>(
        conn: &Connection,
        query: &str,
        limit: usize,
        provider: &P,
        vector_backend: &VectorBackend,
        filter: Option<&str>,
    ) -> Result<QueryResponse> {
        let mut options = QueryOptions::new(limit);
        options.filter = filter
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        Self::query_with_options(conn, query, &options, provider, vector_backend)
    }

    pub fn query_with_options<P: EmbeddingProvider + ?Sized>(
        conn: &Connection,
        query: &str,
        options: &QueryOptions,
        provider: &P,
        vector_backend: &VectorBackend,
    ) -> Result<QueryResponse> {
        let started = Instant::now();
        let limit = options.limit;
        if limit == 0 {
            return Err(anyhow!("--limit must be a positive integer"));
        }
        if let Some(min_score) = options.min_score {
            if !min_score.is_finite() {
                return Err(anyhow!("--min-score must be a finite number"));
            }
        }
        let retrieval_depth = options
            .effective_retrieval_depth()
            .map_err(|err| anyhow!(err))?;
        let query_id = format!(
            "q_{}_{}",
            chrono::Utc::now().timestamp_micros(),
            std::process::id()
        );
        let plan = QueryPlan::analyze(query).with_expansion(options.query_expansion);
        let filter_text = options
            .filter
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let filter_sql = compile_filter(filter_text.as_deref(), "c")?;
        provider.health()?;
        ensure_has_embeddings(conn)?;
        let optimizer_config = optimizer::load_runtime_config(conn).unwrap_or_default();
        let plan = plan.with_runtime_config(&optimizer_config);
        ensure_runtime_profile(
            conn,
            provider,
            provider.dimension(),
            provider.model_id(),
            vector_backend,
        )?;
        let (mut results, mut routing) = match vector_backend {
            VectorBackend::SqliteVec => query_hybrid(
                conn,
                query,
                &plan,
                limit,
                provider,
                filter_sql.as_ref(),
                options.rerank,
            )?,
            VectorBackend::Exact => query_exact(
                conn,
                query,
                &plan,
                limit,
                provider,
                filter_sql.as_ref(),
                options.rerank,
            )?,
        };
        routing.query_expansion = options.query_expansion;
        routing.query_expansion_terms = plan.expanded_terms.clone();
        routing.external_reranker = options.external_reranker;
        if retrieval_depth > 0 {
            let expansion_started = Instant::now();
            routing.link_candidates = expand_link_candidates(
                conn,
                &mut results,
                limit,
                &plan,
                query,
                filter_sql.as_ref(),
                options.rerank,
            )?;
            routing.timings.link_expansion_ms = expansion_started.elapsed().as_millis();
            routing.merged_candidates = results.len();
        }
        apply_temporal_quality(&mut results, options, query)?;
        if options.external_reranker {
            apply_lexical_reranker(&mut results, &plan, query);
        }
        if optimizer::apply_runtime_adjustments(&mut results, &optimizer_config) > 0 {
            sort_search_results(&mut results);
        }
        let filtered_candidates = results.len();
        if let Some(min_score) = options.min_score {
            let before_threshold = results.len();
            results.retain(|result| result.score >= min_score);
            routing.threshold_filtered = Some(before_threshold.saturating_sub(results.len()));
        }
        results.truncate(limit);
        let enrich_started = Instant::now();
        if options.context_chunks > 0 || options.include_links {
            enrich_results(
                conn,
                &mut results,
                options.context_chunks,
                options.include_links,
            )?;
        }
        routing.timings.enrich_ms = enrich_started.elapsed().as_millis();
        for result in &mut results {
            refresh_evidence_count(result);
            refresh_result_summaries(result);
        }
        routing.returned = results.len();
        routing.min_score = options.min_score;
        routing.context_chunks = options.context_chunks;
        routing.include_links = options.include_links;
        routing.expand_links = retrieval_depth;
        routing.retrieval_depth = retrieval_depth;
        if filter_text.is_some() {
            routing.filtered_candidates = Some(filtered_candidates);
            routing.filter_mode = Some(match vector_backend {
                VectorBackend::SqliteVec => "sql_pushdown+filtered_exact_vector".to_string(),
                VectorBackend::Exact => "sql_pushdown".to_string(),
            });
        }
        routing.filter = filter_text;
        routing.timings.total_ms = started.elapsed().as_millis();
        let explain = if options.explain {
            Some(build_query_explain_trace(
                &routing,
                &results,
                vector_backend.as_str(),
                limit,
            ))
        } else {
            None
        };
        let optimizer_status = None;
        Ok(QueryResponse {
            query: query.to_string(),
            query_id,
            took_ms: routing.timings.total_ms,
            mode: routing.strategy.clone(),
            route: plan.route.as_str().to_string(),
            routing,
            vector_backend: vector_backend.as_str().to_string(),
            explain,
            optimizer: optimizer_status,
            results,
        })
    }

    pub fn feedback(conn: &Connection, event: &FeedbackEvent) -> Result<FeedbackResponse> {
        let id = conn.execute(
            "INSERT INTO feedback_events(event, query_id, chunk_id, query, payload, created_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.event,
                event.query_id,
                event.chunk_id,
                event.query,
                event.payload.to_string(),
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(FeedbackResponse {
            ok: true,
            event_id: id as i64,
        })
    }

    pub fn get_chunks(conn: &Connection, options: &ChunkGetOptions) -> Result<ChunkGetResponse> {
        let mut seen = HashSet::new();
        let mut ids = Vec::new();
        for raw in &options.chunk_ids {
            let id = raw.trim();
            if id.is_empty() || !seen.insert(id.to_string()) {
                continue;
            }
            ids.push(id.to_string());
            if ids.len() >= 50 {
                break;
            }
        }

        let mut results = Vec::new();
        for id in ids {
            if let Some(mut result) = load_chunk_by_chunk_id(conn, &id, &options.detail)? {
                let radius = options.context_chunks.min(3);
                if radius > 0 {
                    result.context_chunks = load_context_chunks_for_chunk(
                        conn,
                        &result.path,
                        result.line_start,
                        result.line_end,
                        radius,
                    )?;
                }
                results.push(result);
            }
        }

        Ok(ChunkGetResponse {
            schema_version: "orderk.get.v1".to_string(),
            total: results.len(),
            detail: options.detail.clone(),
            results,
        })
    }
}

fn delete_file(conn: &Connection, path: &str) -> Result<()> {
    let mut stmt = conn.prepare("SELECT id FROM chunks WHERE file_path = ?1")?;
    let row_ids = stmt.query_map(params![path], |row| row.get::<_, i64>(0))?;
    let mut ids = Vec::new();
    for id in row_ids {
        ids.push(id?);
    }
    for id in &ids {
        conn.execute("DELETE FROM vec_chunks WHERE rowid = ?1", params![id])?;
    }
    conn.execute(
        "DELETE FROM fts_chunks WHERE rowid IN (SELECT id FROM chunks WHERE file_path = ?1)",
        params![path],
    )?;
    conn.execute("DELETE FROM chunk_embeddings WHERE chunk_id IN (SELECT chunk_id FROM chunks WHERE file_path = ?1)", params![path])?;
    conn.execute("DELETE FROM chunks WHERE file_path = ?1", params![path])?;
    conn.execute("DELETE FROM files WHERE path = ?1", params![path])?;
    Ok(())
}

fn delete_file_in_tx(tx: &rusqlite::Transaction<'_>, path: &str) -> Result<()> {
    let mut stmt = tx.prepare("SELECT id FROM chunks WHERE file_path = ?1")?;
    let row_ids = stmt.query_map(params![path], |row| row.get::<_, i64>(0))?;
    let mut ids = Vec::new();
    for id in row_ids {
        ids.push(id?);
    }
    for id in &ids {
        tx.execute("DELETE FROM vec_chunks WHERE rowid = ?1", params![id])?;
    }
    tx.execute(
        "DELETE FROM fts_chunks WHERE rowid IN (SELECT id FROM chunks WHERE file_path = ?1)",
        params![path],
    )?;
    tx.execute("DELETE FROM chunk_embeddings WHERE chunk_id IN (SELECT chunk_id FROM chunks WHERE file_path = ?1)", params![path])?;
    tx.execute("DELETE FROM chunks WHERE file_path = ?1", params![path])?;
    tx.execute("DELETE FROM files WHERE path = ?1", params![path])?;
    Ok(())
}

fn reindex_file_with_options<P: EmbeddingProvider + ?Sized>(
    conn: &mut Connection,
    file: &ScannedFile,
    provider: &P,
    embedding_dim: usize,
    embedding_model: &str,
    vector_backend: &VectorBackend,
    chunk_options: &IndexOptions,
) -> Result<ReindexFileSummary> {
    let body = fs::read_to_string(&file.abs_path)
        .with_context(|| format!("read {}", file.abs_path.display()))?;
    let parsed = parse_markdown(&file.path, &body)?;
    let chunk_options = chunk_options.normalized();
    let chunks = chunk_document_with_options(
        &parsed,
        ChunkingOptions {
            max_chars: chunk_options.chunk_max_chars,
            overlap_chars: chunk_options.chunk_overlap_chars,
        },
    );
    let mut records: Vec<Option<EmbeddingRecord>> = vec![None; chunks.len()];
    let mut reused = 0usize;
    let mut missing_inputs = Vec::new();

    let reusable = load_reusable_embeddings(conn, &file.path, embedding_model, embedding_dim)?;
    for (idx, chunk) in chunks.iter().enumerate() {
        if let Some(record) = reusable.get(&chunk.id).cloned() {
            records[idx] = Some(record);
            reused += 1;
        } else {
            missing_inputs.push((idx, chunk_embedding_input(chunk)));
        }
    }

    let mut embedded = 0usize;
    if !missing_inputs.is_empty() {
        let embeddings = provider.embed_documents(
            &missing_inputs
                .iter()
                .map(|(_, text)| text.clone())
                .collect::<Vec<_>>(),
        )?;
        if embeddings.len() != missing_inputs.len() {
            return Err(anyhow!("embedding count mismatch for file {}", file.path));
        }
        for ((idx, _), vector) in missing_inputs.iter().zip(embeddings.iter()) {
            records[*idx] = Some(EmbeddingRecord {
                blob: vector_to_blob(vector),
                vector_hash: vector_hash(vector),
            });
            embedded += 1;
        }
    }

    let tx = conn.transaction()?;
    delete_file_in_tx(&tx, &file.path)?;
    tx.execute(
        "INSERT INTO files(path, mtime, size, hash, indexed_at) VALUES(?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(path) DO UPDATE SET mtime=excluded.mtime, size=excluded.size, hash=excluded.hash, indexed_at=excluded.indexed_at",
        params![file.path, file.mtime, file.size as i64, file.hash, Utc::now().to_rfc3339()],
    )?;
    for (chunk, record) in chunks.iter().zip(records) {
        let record =
            record.ok_or_else(|| anyhow!("missing embedding record for chunk {}", chunk.id))?;
        let tags = serde_json::to_string(&chunk.tags)?;
        tx.execute(
            "INSERT INTO chunks(chunk_id, file_path, file_hash, title, heading, line_start, line_end, text, tags_json, links_json, has_code, has_link, has_task_list, has_incomplete_tasks, confidence, status, source_type, valid_from, valid_until, supersedes, superseded_by, updated, chunk_hash, mtime)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            params![
                chunk.id,
                chunk.file_path,
                file.hash,
                chunk.title,
                chunk.heading,
                chunk.line_start as i64,
                chunk.line_end as i64,
                chunk.text,
                tags,
                serde_json::to_string(&extract_wikilinks_from_text(&chunk.text))?,
                bool_to_i64(chunk.has_code),
                bool_to_i64(chunk.has_link),
                bool_to_i64(chunk.has_task_list),
                bool_to_i64(chunk.has_incomplete_tasks),
                chunk.confidence,
                chunk.status,
                chunk.source_type,
                chunk.valid_from,
                chunk.valid_until,
                chunk.supersedes,
                chunk.superseded_by,
                chunk.updated,
                chunk.hash,
                file.mtime,
            ],
        )?;
        let rowid = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO fts_chunks(rowid, chunk_id, file_path, title, heading, text, tags) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![rowid, chunk.id, chunk.file_path, chunk.title, chunk.heading, chunk.text, serde_json::to_string(&chunk.tags)?],
        )?;
        tx.execute(
            "INSERT INTO chunk_embeddings(chunk_id, model, dim, embedding, vector_hash) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![chunk.id, embedding_model, embedding_dim as i64, record.blob.clone(), record.vector_hash],
        )?;
        tx.execute(
            "INSERT INTO vec_chunks(rowid, embedding) VALUES(?1, ?2)",
            params![rowid, record.blob],
        )?;
    }
    tx.commit()?;
    let _ = vector_backend;
    Ok(ReindexFileSummary {
        chunks: chunks.len(),
        embedded,
        reused,
    })
}

fn load_reusable_embeddings(
    conn: &Connection,
    file_path: &str,
    model: &str,
    dim: usize,
) -> Result<HashMap<String, EmbeddingRecord>> {
    let mut stmt = conn.prepare(
        "SELECT c.chunk_id, e.embedding, e.vector_hash
         FROM chunks c
         JOIN chunk_embeddings e ON e.chunk_id = c.chunk_id
         WHERE c.file_path = ?1 AND e.model = ?2 AND e.dim = ?3",
    )?;
    let rows = stmt.query_map(params![file_path, model, dim as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            EmbeddingRecord {
                blob: row.get::<_, Vec<u8>>(1)?,
                vector_hash: row.get::<_, String>(2)?,
            },
        ))
    })?;
    let mut map = HashMap::new();
    for row in rows {
        let (chunk_id, record) = row?;
        map.insert(chunk_id, record);
    }
    Ok(map)
}

fn chunk_embedding_input(chunk: &Chunk) -> String {
    let mut parts = Vec::new();
    if let Some(title) = &chunk.title {
        if !title.trim().is_empty() {
            parts.push(format!("title: {}", title.trim()));
        }
    }
    if let Some(heading) = &chunk.heading {
        if !heading.trim().is_empty() {
            parts.push(format!("heading: {}", heading.trim()));
        }
    }
    if !chunk.tags.is_empty() {
        parts.push(format!("tags: {}", chunk.tags.join(" ")));
    }
    parts.push(chunk.text.clone());
    parts.join("\n")
}

fn clean_route_term(term: &str) -> String {
    term.trim()
        .trim_start_matches("path:")
        .trim_start_matches("tag:")
        .trim_start_matches('#')
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '(' || c == ')')
        .to_lowercase()
}

fn route_terms(plan: &QueryPlan) -> Vec<String> {
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

fn add_reason(hit: &mut RouteHit, label: &str, score: f32) {
    if !hit.reasons.iter().any(|reason| reason == label) {
        hit.score += score;
        hit.reasons.push(label.to_string());
    }
}

fn score_route_hit(
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

fn append_filter_clause(sql: &mut String, filter: Option<&FilterSql>) {
    if let Some(filter) = filter {
        sql.push_str(" AND ");
        sql.push_str(&filter.sql);
    }
}

fn append_filter_args(args: &mut Vec<Value>, filter: Option<&FilterSql>) {
    if let Some(filter) = filter {
        args.extend(filter.args.iter().cloned());
    }
}

fn collect_route_hits(
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

fn sqlite_vec_vector_scores(
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

fn filtered_exact_vector_scores(
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

fn query_hybrid<P: EmbeddingProvider + ?Sized>(
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

fn query_exact<P: EmbeddingProvider + ?Sized>(
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
fn load_chunk_result(
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

fn parse_orderk_date(value: Option<&str>) -> Option<NaiveDate> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    NaiveDate::parse_from_str(value.get(..10).unwrap_or(value), "%Y-%m-%d").ok()
}

fn query_has_recent_cue(query: &str) -> bool {
    let q = query.to_lowercase();
    [
        "recent", "recently", "latest", "current", "today", "现在", "最新", "当前",
    ]
    .iter()
    .any(|cue| q.contains(cue))
}

fn query_has_oldest_cue(query: &str) -> bool {
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

fn confidence_boost_value(confidence: Option<&str>) -> f32 {
    match confidence.unwrap_or("").trim().to_lowercase().as_str() {
        "verified" | "high" => 0.025,
        "observed" | "medium" => 0.012,
        "inferred" | "low" => -0.01,
        "stale" => -0.03,
        _ => 0.0,
    }
}

fn status_boost_value(status: Option<&str>, state: &str) -> f32 {
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

fn evidence_count_boost_value(result: &SearchResult) -> f32 {
    let count = result.evidence.sources.len().min(4) as f32;
    if count > 0.0 {
        0.005 * count
    } else {
        0.0
    }
}

fn freshness_boost_value(
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

fn build_validity(result: &SearchResult, as_of: Option<NaiveDate>) -> ValidityEvidence {
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

fn result_has_temporal_quality_metadata(result: &SearchResult) -> bool {
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

fn apply_temporal_quality(
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

fn quality_summary(breakdown: &ScoreBreakdown) -> QualitySummary {
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

fn evidence_summary(result: &SearchResult) -> EvidenceSummary {
    EvidenceSummary {
        schema_version: "orderk.evidence_summary.v1".to_string(),
        validity_state: result.validity.state.clone(),
        stale_reason: result.validity.stale_reason.clone(),
        age_days: result.validity.age_days,
        confidence: result.confidence.clone(),
        status: result.status.clone(),
        source_type: result.source_type.clone(),
        evidence_count: result.evidence.evidence_count,
        sources: result.evidence.sources.clone(),
        evidence_uri: result.evidence_uri.clone(),
        open_uri: result.open_uri.clone(),
    }
}

fn refresh_result_summaries(result: &mut SearchResult) {
    result.quality = quality_summary(&result.score_breakdown);
    result.evidence_summary = evidence_summary(result);
}

fn sort_search_results(results: &mut [SearchResult]) {
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line_start.cmp(&b.line_start))
            .then_with(|| a.chunk_id.cmp(&b.chunk_id))
    });
}

fn apply_lexical_reranker(results: &mut [SearchResult], plan: &QueryPlan, query: &str) {
    let mut terms = plan.all_terms();
    terms.extend(
        normalize_query(query)
            .split_whitespace()
            .map(ToString::to_string),
    );
    terms.retain(|term| term.chars().count() >= 2);
    terms.sort();
    terms.dedup();
    if terms.is_empty() {
        return;
    }
    let normalized_query = normalize_query(query).to_lowercase();
    for result in results.iter_mut() {
        let haystack = format!(
            "{} {} {} {} {}",
            result.path,
            result.title.as_deref().unwrap_or(""),
            result.heading.as_deref().unwrap_or(""),
            result.snippet,
            result.tags.join(" ")
        )
        .to_lowercase();
        let matched = terms
            .iter()
            .filter(|term| haystack.contains(term.as_str()))
            .count();
        if matched == 0 {
            continue;
        }
        let coverage = matched as f32 / terms.len() as f32;
        let phrase_boost = if !normalized_query.is_empty() && haystack.contains(&normalized_query) {
            0.02
        } else {
            0.0
        };
        let boost = (coverage * 0.06 + phrase_boost).min(0.08);
        if boost <= 0.0 {
            continue;
        }
        result.score += boost;
        result.score_breakdown.reranker_boost = boost;
        if !result
            .evidence
            .sources
            .iter()
            .any(|source| source == "lexical_reranker")
        {
            result.evidence.sources.push("lexical_reranker".to_string());
        }
        refresh_evidence_count(result);
        refresh_result_summaries(result);
    }
    sort_search_results(results);
}

fn expand_link_candidates(
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

fn apply_link_expansion_signal(result: &mut SearchResult) -> bool {
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

fn outgoing_link_rowids(conn: &Connection, seed: &SearchResult) -> Result<Vec<i64>> {
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

fn backlink_rowids(conn: &Connection, seed: &SearchResult) -> Result<Vec<i64>> {
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

fn refresh_evidence_count(result: &mut SearchResult) {
    let link_count = result
        .evidence
        .links
        .as_ref()
        .map(|links| links.outgoing.len() + links.backlinks.len())
        .unwrap_or(0);
    result.evidence.evidence_count = result.evidence.sources.len() + link_count;
}

fn enrich_results(
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

fn load_chunk_by_chunk_id(
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

fn load_context_chunks_for_chunk(
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

fn load_context_chunks(
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

fn load_link_evidence(conn: &Connection, result: &SearchResult) -> Result<LinkEvidence> {
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
fn build_result(
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
    };
    let score = (keyword_score * 0.35)
        + (vector_score * 0.35)
        + fusion
        + breakdown.path_boost
        + breakdown.tag_boost
        + breakdown.route_boost
        + breakdown.recency_boost
        + metadata_boost
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
        validity: ValidityEvidence::default(),
        valid_from,
        valid_until,
        supersedes,
        superseded_by,
        updated,
        mtime: mtime.and_then(|ts| Utc.timestamp_opt(ts, 0).single()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::MockEmbeddingProvider;
    use std::fs;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample_vault() -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dir.push(format!("orderk-index-{}-{}", std::process::id(), unique));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("alpha.md"), "---\ntags: [project, alpha]\n---\n# Alpha Project\nThe alpha project uses sqlite-vec local semantic search.\nIt includes chunking and FTS.\n").unwrap();
        fs::write(
            dir.join("bravo.md"),
            "# Bravo Project\nObsidian plugin packaging and npm workspace builds.",
        )
        .unwrap();
        dir
    }

    #[test]
    fn open_uri_percent_encodes_path_query_component() {
        assert_eq!(
            open_uri("dir/a b&c?.md", 12),
            "obsidian://open?path=dir%2Fa%20b%26c%3F.md&line=12"
        );
    }

    #[derive(Debug, Clone)]
    struct CountingMockEmbeddingProvider {
        inner: MockEmbeddingProvider,
        calls: Arc<AtomicUsize>,
        total_inputs: Arc<AtomicUsize>,
    }

    impl CountingMockEmbeddingProvider {
        fn new(dim: usize) -> Self {
            Self {
                inner: MockEmbeddingProvider::new(dim),
                calls: Arc::new(AtomicUsize::new(0)),
                total_inputs: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn total_inputs(&self) -> usize {
            self.total_inputs.load(Ordering::SeqCst)
        }
    }

    impl EmbeddingProvider for CountingMockEmbeddingProvider {
        fn provider_id(&self) -> &str {
            self.inner.provider_id()
        }

        fn model_id(&self) -> &str {
            self.inner.model_id()
        }

        fn dimension(&self) -> usize {
            self.inner.dimension()
        }

        fn health(&self) -> Result<()> {
            self.inner.health()
        }

        fn embed_documents(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.total_inputs.fetch_add(inputs.len(), Ordering::SeqCst);
            self.inner.embed_documents(inputs)
        }
    }

    #[test]
    fn index_and_query_roundtrip() {
        let vault = sample_vault();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-db-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let summary = IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(summary.files, 2);
        let res = IndexStore::query(
            &conn,
            "sqlite vec semantic search",
            5,
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert!(!res.results.is_empty());
        assert_eq!(res.results[0].path, "alpha.md");
        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn get_chunks_preserves_order_dedupes_skips_missing_and_caps_batches() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-get-bounds-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        for idx in 0..55 {
            fs::write(
                vault.join(format!("note-{idx:02}.md")),
                format!("# Note {idx:02}\nCompact recall get boundary text {idx:02}.\n"),
            )
            .unwrap();
        }
        let db_path = std::env::temp_dir().join(format!(
            "orderk-get-bounds-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        let mut stmt = conn
            .prepare("SELECT chunk_id FROM chunks ORDER BY file_path ASC")
            .unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        let ids = rows.map(|row| row.unwrap()).collect::<Vec<_>>();
        assert!(ids.len() >= 55);

        let mut requested = vec![ids[2].clone(), ids[0].clone(), ids[2].clone()];
        requested.extend(ids.iter().cloned());
        let response = IndexStore::get_chunks(
            &conn,
            &ChunkGetOptions {
                chunk_ids: requested,
                detail: ChunkGetDetail::Full,
                context_chunks: 0,
            },
        )
        .unwrap();
        assert_eq!(response.schema_version, "orderk.get.v1");
        assert_eq!(response.total, 50);
        assert_eq!(response.results[0].chunk_id, ids[2]);
        assert_eq!(response.results[1].chunk_id, ids[0]);
        assert_eq!(
            response
                .results
                .iter()
                .filter(|result| result.chunk_id == ids[2])
                .count(),
            1
        );
        assert!(response.results[0]
            .text
            .contains("Compact recall get boundary text"));
        assert!(response.results[0]
            .evidence_uri
            .starts_with("orderk://chunk/"));
        assert_eq!(
            response.results[0].open_uri,
            open_uri(&response.results[0].path, response.results[0].line_start)
        );

        let missing = IndexStore::get_chunks(
            &conn,
            &ChunkGetOptions {
                chunk_ids: vec!["missing".to_string(), ids[1].clone()],
                detail: ChunkGetDetail::Full,
                context_chunks: 0,
            },
        )
        .unwrap();
        assert_eq!(missing.total, 1);
        assert_eq!(missing.results[0].chunk_id, ids[1]);

        let summary = IndexStore::get_chunks(
            &conn,
            &ChunkGetOptions {
                chunk_ids: vec![ids[0].clone()],
                detail: ChunkGetDetail::Summary,
                context_chunks: 0,
            },
        )
        .unwrap();
        assert_eq!(summary.results.len(), 1);
        assert!(summary.results[0].text.len() <= 180);

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_explain_trace_is_opt_in_and_mechanical() {
        let vault = sample_vault();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-explain-trace-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();

        let mut options = QueryOptions::new(5);
        let without_explain = IndexStore::query_with_options(
            &conn,
            "sqlite vec semantic search",
            &options,
            &provider,
            &VectorBackend::Exact,
        )
        .unwrap();
        assert!(without_explain.explain.is_none());

        options.explain = true;
        let with_explain = IndexStore::query_with_options(
            &conn,
            "sqlite vec semantic search",
            &options,
            &provider,
            &VectorBackend::Exact,
        )
        .unwrap();
        let explain = with_explain
            .explain
            .expect("--explain should include a trace");
        assert_eq!(explain.schema_version, "orderk.explain_trace.v1");
        assert!(explain.stages.iter().any(|stage| stage.name == "vector"));
        assert!(explain.stages.iter().any(|stage| stage.name == "merge"));
        assert_eq!(explain.returned, with_explain.results.len());
        assert_eq!(explain.route, with_explain.route);

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn index_persists_chunk_structural_metadata() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-metadata-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("metadata.md"),
            "# Metadata\n- [ ] ship filter DSL\nSee https://example.com\n```rust\nfn main() {}\n```\n",
        ).unwrap();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-metadata-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let row = conn.query_row(
            "SELECT has_code, has_link, has_task_list, has_incomplete_tasks FROM chunks WHERE file_path = 'metadata.md' LIMIT 1",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
        ).unwrap();
        assert_eq!(row, (1, 1, 1, 1));
        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn init_schema_migrates_and_backfills_old_chunk_metadata_columns() {
        let db_path = std::env::temp_dir().join(format!(
            "orderk-old-schema-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        register_sqlite_vec();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE chunks (
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
                chunk_hash TEXT NOT NULL,
                mtime INTEGER NOT NULL
            );
            "#,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks(chunk_id, file_path, file_hash, title, heading, line_start, line_end, text, tags_json, chunk_hash, mtime)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                "chk_old",
                "old.md",
                "filehash",
                "Old",
                "Old",
                1_i64,
                4_i64,
                "- [ ] task\nhttps://example.com\n```rust\nfn main() {}\n```",
                "[\"rust\"]",
                "chunkhash",
                1_i64,
            ],
        ).unwrap();
        init_schema(&conn, 8, "mock-8", &VectorBackend::SqliteVec).unwrap();
        assert!(chunk_column_exists(&conn, "has_code").unwrap());
        assert!(chunk_column_exists(&conn, "has_link").unwrap());
        assert!(chunk_column_exists(&conn, "has_task_list").unwrap());
        assert!(chunk_column_exists(&conn, "has_incomplete_tasks").unwrap());
        let row = conn.query_row(
            "SELECT has_code, has_link, has_task_list, has_incomplete_tasks FROM chunks WHERE chunk_id = 'chk_old'",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
        ).unwrap();
        assert_eq!(row, (1, 1, 1, 1));
        init_schema(&conn, 8, "mock-8", &VectorBackend::SqliteVec).unwrap();
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn migration_backfills_existing_metadata_columns_when_schema_version_missing() {
        let db_path = std::env::temp_dir().join(format!(
            "orderk-partial-schema-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE chunks (
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
                has_code INTEGER NOT NULL DEFAULT 0,
                has_link INTEGER NOT NULL DEFAULT 0,
                has_task_list INTEGER NOT NULL DEFAULT 0,
                has_incomplete_tasks INTEGER NOT NULL DEFAULT 0,
                chunk_hash TEXT NOT NULL,
                mtime INTEGER NOT NULL
            );
            "#,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks(chunk_id, file_path, file_hash, title, heading, line_start, line_end, text, tags_json, chunk_hash, mtime)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params!["chk_partial", "partial.md", "filehash", "Partial", "Partial", 1_i64, 4_i64, "- [ ] task\nhttps://example.com\n```rust\nfn main() {}\n```", "[]", "chunkhash", 1_i64],
        ).unwrap();
        migrate_chunk_metadata_columns(&conn).unwrap();
        let row = conn.query_row(
            "SELECT has_code, has_link, has_task_list, has_incomplete_tasks FROM chunks WHERE chunk_id = 'chk_partial'",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
        ).unwrap();
        assert_eq!(row, (1, 1, 1, 1));
        let schema_version: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(schema_version, "5");
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn public_query_refuses_old_schema_without_migrating_read_only_surface() {
        let db_path = std::env::temp_dir().join(format!(
            "orderk-public-query-old-schema-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        register_sqlite_vec();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO settings(key, value) VALUES
              ('embedding_provider', 'mock'),
              ('embedding_model', 'mock-8'),
              ('embedding_dim', '8'),
              ('vector_backend', 'exact'),
              ('vector_backend_mode', 'exact'),
              ('schema_version', '3');
            CREATE TABLE files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                mtime INTEGER NOT NULL,
                size INTEGER NOT NULL,
                hash TEXT NOT NULL,
                indexed_at TEXT NOT NULL
            );
            CREATE TABLE chunks (
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
                chunk_hash TEXT NOT NULL,
                mtime INTEGER NOT NULL
            );
            CREATE TABLE chunk_embeddings (
                chunk_id TEXT PRIMARY KEY,
                model TEXT NOT NULL,
                dim INTEGER NOT NULL,
                embedding BLOB NOT NULL,
                vector_hash TEXT NOT NULL
            );
            CREATE VIRTUAL TABLE fts_chunks USING fts5(
                chunk_id UNINDEXED,
                file_path UNINDEXED,
                title,
                heading,
                text,
                tags,
                tokenize = 'unicode61 remove_diacritics 2'
            );
            "#,
        )
        .unwrap();
        let provider = MockEmbeddingProvider::new(8);
        let text = "legacy orderk search migration evidence";
        let embedding = vector_to_blob(&provider.embed_query(text).unwrap());
        conn.execute(
            "INSERT INTO files(path, mtime, size, hash, indexed_at) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![
                "legacy.md",
                1_i64,
                text.len() as i64,
                "filehash",
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks(chunk_id, file_path, file_hash, title, heading, line_start, line_end, text, tags_json, links_json, has_code, has_link, has_task_list, has_incomplete_tasks, chunk_hash, mtime)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params!["legacy_1", "legacy.md", "filehash", "Legacy", "Legacy", 1_i64, 1_i64, text, "[\"orderk\"]", "[]", 0_i64, 0_i64, 0_i64, 0_i64, "chunkhash", 1_i64],
        ).unwrap();
        conn.execute(
            "INSERT INTO chunk_embeddings(chunk_id, model, dim, embedding, vector_hash) VALUES(?1, ?2, ?3, ?4, ?5)",
            params!["legacy_1", "mock-8", 8_i64, embedding, "vectorhash"],
        ).unwrap();
        conn.execute(
            "INSERT INTO fts_chunks(rowid, chunk_id, file_path, title, heading, text, tags) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![1_i64, "legacy_1", "legacy.md", "Legacy", "Legacy", text, "orderk"],
        ).unwrap();
        drop(conn);

        let err = crate::api::query_with_options(
            &db_path,
            "orderk migration",
            &QueryOptions::new(5),
            &provider,
            VectorBackend::Exact,
        )
        .expect_err("public search must not mutate legacy DBs to migrate schema");
        assert!(
            err.to_string().contains("schema") || err.to_string().contains("confidence"),
            "unexpected read-only schema error: {err:#}"
        );
        assert!(
            !chunk_column_exists(&rusqlite::Connection::open(&db_path).unwrap(), "confidence")
                .unwrap(),
            "read-only search must not add missing columns"
        );
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_filter_limits_results_to_matching_chunk_metadata() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-filter-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("code.md"),
            "---\ntags: [keep, rust]\nconfidence: high\nstatus: active\nsource_type: audit\n---\n# Filter Code\nshared retrieval needle\n```rust\nfn main() {}\n```\n",
        ).unwrap();
        fs::write(
            vault.join("plain.md"),
            "---\ntags: [drop]\n---\n# Filter Plain\nshared retrieval needle without code\n",
        )
        .unwrap();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-filter-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let res = IndexStore::query_with_filter(
            &conn,
            "shared retrieval needle",
            10,
            &provider,
            &VectorBackend::SqliteVec,
            Some("tag == 'keep' && has_code == true && confidence == 'high' && status == 'active' && source_type == 'audit'"),
        )
        .unwrap();
        assert!(!res.results.is_empty());
        assert!(
            res.results.iter().all(|r| r.path == "code.md"),
            "{:#?}",
            res.results
        );
        assert!(res
            .results
            .iter()
            .all(|r| r.confidence.as_deref() == Some("high")));
        assert!(res
            .results
            .iter()
            .all(|r| r.status.as_deref() == Some("active")));
        assert!(res
            .results
            .iter()
            .all(|r| r.source_type.as_deref() == Some("audit")));
        assert!(res
            .results
            .iter()
            .all(|r| r.score_breakdown.metadata_boost > 0.0));
        assert_eq!(
            res.routing.filter,
            Some("tag == 'keep' && has_code == true && confidence == 'high' && status == 'active' && source_type == 'audit'".to_string())
        );
        assert_eq!(res.routing.filtered_candidates, Some(res.results.len()));
        assert_eq!(
            res.routing.filter_mode.as_deref(),
            Some("sql_pushdown+filtered_exact_vector")
        );

        let none = IndexStore::query_with_filter(
            &conn,
            "shared retrieval needle",
            10,
            &provider,
            &VectorBackend::SqliteVec,
            Some("tag == 'missing'"),
        )
        .unwrap();
        assert!(none.results.is_empty(), "{:#?}", none.results);

        let err = IndexStore::query_with_filter(
            &conn,
            "shared retrieval needle",
            10,
            &provider,
            &VectorBackend::SqliteVec,
            Some("unknown == 'x'"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid filter"), "{err:?}");

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_filter_uses_exact_tag_membership_not_substring() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-filter-tags-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("rust.md"),
            "---\ntags: [rust]\n---\n# Rust\nshared tag needle\n",
        )
        .unwrap();
        fs::write(
            vault.join("rustic.md"),
            "---\ntags: [rustic]\n---\n# Rustic\nshared tag needle\n",
        )
        .unwrap();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-filter-tags-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let res = IndexStore::query_with_filter(
            &conn,
            "shared tag needle",
            10,
            &provider,
            &VectorBackend::SqliteVec,
            Some("tag == 'rust'"),
        )
        .unwrap();
        assert_eq!(
            res.results
                .iter()
                .map(|r| r.path.as_str())
                .collect::<Vec<_>>(),
            vec!["rust.md"]
        );

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_filter_applies_to_exact_backend_and_none_preserves_search() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-filter-exact-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("code.md"),
            "# Code\nexact backend needle\n```rust\nfn main() {}\n```\n",
        )
        .unwrap();
        fs::write(
            vault.join("plain.md"),
            "# Plain\nexact backend needle without code\n",
        )
        .unwrap();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-filter-exact-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();

        let no_filter = IndexStore::query(
            &conn,
            "exact backend needle",
            10,
            &provider,
            &VectorBackend::Exact,
        )
        .unwrap();
        let none_filter = IndexStore::query_with_filter(
            &conn,
            "exact backend needle",
            10,
            &provider,
            &VectorBackend::Exact,
            None,
        )
        .unwrap();
        assert_eq!(no_filter.results.len(), none_filter.results.len());
        assert_eq!(
            no_filter.results.first().map(|r| &r.path),
            none_filter.results.first().map(|r| &r.path)
        );

        let filtered = IndexStore::query_with_filter(
            &conn,
            "exact backend needle",
            10,
            &provider,
            &VectorBackend::Exact,
            Some("has_code == false"),
        )
        .unwrap();
        assert!(!filtered.results.is_empty());
        assert!(
            filtered.results.iter().all(|r| r.path == "plain.md"),
            "{:#?}",
            filtered.results
        );
        assert_eq!(
            filtered.routing.filtered_candidates,
            Some(filtered.results.len())
        );
        assert_eq!(
            filtered.routing.filter_mode.as_deref(),
            Some("sql_pushdown")
        );

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn temporal_filter_fields_apply_before_result_truncation() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-temporal-filter-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("current.md"),
            "---
valid_from: 2026-05-01
updated: 2026-05-18
status: active
confidence: high
---
# Current
shared temporal filter needle current evidence
",
        )
        .unwrap();
        fs::write(
            vault.join("old.md"),
            "---
valid_from: 2026-04-01
updated: 2026-04-15
superseded_by: current.md
status: stale
confidence: low
---
# Old
shared temporal filter needle old evidence
",
        )
        .unwrap();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-temporal-filter-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();

        let current = IndexStore::query_with_filter(
            &conn,
            "shared temporal filter needle old current",
            1,
            &provider,
            &VectorBackend::Exact,
            Some("valid_from == '2026-05-01' && updated contains '2026-05'"),
        )
        .unwrap();
        assert_eq!(
            current
                .results
                .iter()
                .map(|r| r.path.as_str())
                .collect::<Vec<_>>(),
            vec!["current.md"]
        );

        let mut old_options = QueryOptions::new(1);
        old_options.filter = Some("superseded_by == 'current.md'".to_string());
        old_options.include_stale = true;
        let old = IndexStore::query_with_options(
            &conn,
            "shared temporal filter needle old current",
            &old_options,
            &provider,
            &VectorBackend::Exact,
        )
        .unwrap();
        assert_eq!(
            old.results
                .iter()
                .map(|r| r.path.as_str())
                .collect::<Vec<_>>(),
            vec!["old.md"]
        );

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_routes_short_path_and_tag_queries() {
        let vault = sample_vault();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-routing-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let path_res = IndexStore::query(
            &conn,
            "path:alpha.md",
            5,
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(path_res.route, "path");
        assert!(path_res
            .routing
            .routes_attempted
            .iter()
            .any(|route| route == "path"));
        assert_eq!(path_res.results[0].path, "alpha.md");
        assert_eq!(path_res.results[0].evidence.route.as_deref(), Some("path"));
        assert!(path_res.results[0].evidence.route_score > 0.0);
        assert!(path_res.results[0]
            .evidence
            .sources
            .iter()
            .any(|source| source == "path"));

        let tag_res =
            IndexStore::query(&conn, "#alpha", 5, &provider, &VectorBackend::SqliteVec).unwrap();
        assert_eq!(tag_res.route, "tag");
        assert!(tag_res
            .routing
            .routes_attempted
            .iter()
            .any(|route| route == "tag"));
        assert_eq!(tag_res.results[0].path, "alpha.md");
        assert!(tag_res.results[0]
            .evidence
            .sources
            .iter()
            .any(|source| source == "tag"));
        assert!(tag_res.results[0].score_breakdown.route_boost > 0.0);

        let short_res =
            IndexStore::query(&conn, "alpha", 5, &provider, &VectorBackend::SqliteVec).unwrap();
        assert_eq!(short_res.route, "short");
        assert!(short_res
            .routing
            .routes_attempted
            .iter()
            .any(|route| route == "path"));
        assert_eq!(short_res.results[0].path, "alpha.md");

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn reindex_reuses_unchanged_chunk_embeddings() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-reuse-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("reuse.md"),
            "# Alpha\nunchanged alpha body\n## Beta\nold beta body\n",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-reuse-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = CountingMockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let first = IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(first.chunks, 2);
        assert_eq!(first.embedded, 2);
        assert_eq!(first.reused, 0);
        assert_eq!(provider.calls(), 1);
        assert_eq!(provider.total_inputs(), 2);

        fs::write(
            vault.join("reuse.md"),
            "# Alpha\nunchanged alpha body\n## Beta\nnew beta body\n",
        )
        .unwrap();
        let second = IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(second.updated, 1);
        assert_eq!(second.chunks, 2);
        assert_eq!(second.embedded, 1);
        assert_eq!(second.reused, 1);
        assert_eq!(provider.calls(), 2);
        assert_eq!(provider.total_inputs(), 3);

        let res = IndexStore::query(
            &conn,
            "new beta body",
            5,
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(res.results[0].path, "reuse.md");

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn snippet_handles_utf8_boundaries() {
        let text = "前缀🙂这里有中文和 emoji，search 命中后不该 panic，结尾还有更多文本。";
        let out = snippet(text, "中文");
        assert!(out.contains("中文"), "{out}");
        assert!(!out.is_empty());
    }

    #[test]
    fn snippet_preserves_case_and_joins_multiple_query_windows() {
        let text = "Alpha starts with OriginalCase near the beginning.\nThis middle section is deliberately long so the second query term needs a separate snippet window with enough distance between hits.\nLater Beta appears with Emoji🙂 and Markdown.";
        let out = snippet(text, "alpha beta");
        assert!(out.contains("Alpha starts with OriginalCase"), "{out}");
        assert!(out.contains("Beta appears with Emoji🙂"), "{out}");
        assert!(!out.contains("originalcase"), "{out}");
        assert!(out.contains(" … "), "{out}");
        assert!(!out.contains('\n'), "{out}");
    }

    #[test]
    fn snippet_no_hit_fallback_preserves_original_text() {
        let text =
            "# Title\nNo matching term here, but CaseShouldStay and emoji🙂 should remain stable.";
        let out = snippet(text, "absent");
        assert!(out.starts_with("# Title No matching term"), "{out}");
        assert!(out.contains("CaseShouldStay"), "{out}");
        assert!(out.contains("emoji🙂"), "{out}");
        assert!(!out.contains('\n'), "{out}");
    }

    #[derive(Debug, Clone)]
    struct FailingSameProfileProvider;

    impl EmbeddingProvider for FailingSameProfileProvider {
        fn provider_id(&self) -> &str {
            "mock"
        }

        fn model_id(&self) -> &str {
            "mock-8"
        }

        fn dimension(&self) -> usize {
            8
        }

        fn health(&self) -> Result<()> {
            Ok(())
        }

        fn embed_documents(&self, _inputs: &[String]) -> Result<Vec<Vec<f32>>> {
            Err(anyhow!("forced embedding failure"))
        }
    }

    #[test]
    fn failed_reindex_preserves_existing_vectors() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-failed-reindex-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("alpha.md"),
            "# Alpha\nDurable sqlite vec semantic search should survive failed refresh.",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-failed-reindex-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let before = IndexStore::status(&conn).unwrap();
        assert_eq!(before.notes, 1);
        assert_eq!(before.chunks, 1);
        assert_eq!(before.embeddings, 1);

        fs::write(
            vault.join("alpha.md"),
            "# Alpha\nChanged content that would require a fresh embedding.",
        )
        .unwrap();
        let err = IndexStore::index_vault(
            &mut conn,
            &vault,
            &FailingSameProfileProvider,
            8,
            "mock-8",
            &VectorBackend::SqliteVec,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("forced embedding failure"),
            "{err:?}"
        );

        let after = IndexStore::status(&conn).unwrap();
        assert_eq!(after.notes, before.notes);
        assert_eq!(after.chunks, before.chunks);
        assert_eq!(after.embeddings, before.embeddings);
        let res = IndexStore::query(
            &conn,
            "durable sqlite vec semantic search",
            5,
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(res.results[0].path, "alpha.md");

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[derive(Debug, Clone)]
    struct ControlledVectorProvider;

    impl EmbeddingProvider for ControlledVectorProvider {
        fn provider_id(&self) -> &str {
            "controlled-vector"
        }

        fn model_id(&self) -> &str {
            "controlled-4"
        }

        fn dimension(&self) -> usize {
            4
        }

        fn health(&self) -> Result<()> {
            Ok(())
        }

        fn embed_documents(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(inputs
                .iter()
                .map(|input| {
                    let lower = input.to_lowercase();
                    if lower.contains("vector-target-signature")
                        || lower.contains("needle-nearest-neighbor")
                    {
                        vec![1.0, 0.0, 0.0, 0.0]
                    } else if lower.contains("vector-decoy-signature") {
                        vec![0.0, 1.0, 0.0, 0.0]
                    } else {
                        vec![0.0, 0.0, 1.0, 0.0]
                    }
                })
                .collect())
        }
    }

    #[derive(Debug, Clone)]
    struct FilteredVectorProvider;

    impl EmbeddingProvider for FilteredVectorProvider {
        fn provider_id(&self) -> &str {
            "filtered-vector"
        }

        fn model_id(&self) -> &str {
            "filtered-4"
        }

        fn dimension(&self) -> usize {
            4
        }

        fn health(&self) -> Result<()> {
            Ok(())
        }

        fn embed_documents(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(inputs
                .iter()
                .map(|input| {
                    let lower = input.to_lowercase();
                    if lower.contains("needle-nearest-neighbor")
                        || lower.contains("unfiltered-near-decoy")
                    {
                        vec![1.0, 0.0, 0.0, 0.0]
                    } else if lower.contains("filtered-far-target") {
                        vec![0.0, 1.0, 0.0, 0.0]
                    } else {
                        vec![0.0, 0.0, 1.0, 0.0]
                    }
                })
                .collect())
        }
    }

    #[test]
    fn sqlite_vec_filter_high_selectivity_does_not_truncate_before_filtering() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-filter-vector-window-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        for i in 0..20 {
            fs::write(
                vault.join(format!("decoy-{i}.md")),
                format!("---\ntags: [drop]\n---\n# Decoy {i}\nunfiltered-near-decoy {i}\n"),
            )
            .unwrap();
        }
        fs::write(vault.join("target.md"), "---\ntags: [keep]\n---\n# Target\nfiltered-far-target lives here without query keywords.\n").unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-filter-vector-window-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = FilteredVectorProvider;
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let res = IndexStore::query_with_filter(
            &conn,
            "needle-nearest-neighbor",
            2,
            &provider,
            &VectorBackend::SqliteVec,
            Some("tag == 'keep'"),
        )
        .unwrap();
        assert_eq!(
            res.results
                .iter()
                .map(|r| r.path.as_str())
                .collect::<Vec<_>>(),
            vec!["target.md"]
        );
        assert_eq!(
            res.routing.filter_mode.as_deref(),
            Some("sql_pushdown+filtered_exact_vector")
        );

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn sqlite_vec_backend_returns_vector_only_match_without_keyword_hit() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-vector-only-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("target.md"),
            "# Target\nvector-target-signature lives in this unrelated note.",
        )
        .unwrap();
        fs::write(
            vault.join("decoy.md"),
            "# Decoy\nvector-decoy-signature lives in another unrelated note.",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-vector-only-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = ControlledVectorProvider;
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let status = IndexStore::status(&conn).unwrap();
        assert!(
            status.vector_enabled,
            "sqlite-vec extension/table must be enabled"
        );

        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let res = IndexStore::query(
            &conn,
            "needle-nearest-neighbor",
            2,
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(res.vector_backend, "sqlite_vec");
        assert_eq!(res.results[0].path, "target.md");
        assert_eq!(res.results[0].score_breakdown.keyword, 0.0);
        assert!(res.results[0].score_breakdown.vector > res.results[1].score_breakdown.vector);

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_options_apply_temporal_validity_and_quality_breakdown() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-temporal-quality-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("current.md"),
            "---\ntags: [temporal]\nstatus: active\nconfidence: high\nvalid_from: 2026-05-01\nupdated: 2026-05-18\nsource_type: decision\n---\n# Current\ntemporal-quality needle says use the current active policy.\n",
        )
        .unwrap();
        fs::write(
            vault.join("stale.md"),
            "---\ntags: [temporal]\nstatus: stale\nconfidence: low\nvalid_from: 2026-04-01\nvalid_until: 2026-05-01\nsuperseded_by: current.md\nupdated: 2026-04-15\nsource_type: decision\n---\n# Stale\ntemporal-quality needle says use the old superseded policy.\n",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-temporal-quality-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let default = IndexStore::query_with_options(
            &conn,
            "temporal-quality needle current",
            &QueryOptions::new(10),
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(
            default
                .results
                .iter()
                .map(|result| result.path.as_str())
                .collect::<Vec<_>>(),
            vec!["current.md"]
        );
        let exact_db_path = std::env::temp_dir().join(format!(
            "orderk-temporal-quality-exact-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let mut exact_conn = open_db(
            &exact_db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut exact_conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        let limited_default = IndexStore::query_with_options(
            &exact_conn,
            "temporal-quality needle old superseded",
            &QueryOptions {
                limit: 1,
                ..QueryOptions::new(1)
            },
            &provider,
            &VectorBackend::Exact,
        )
        .unwrap();
        assert_eq!(
            limited_default
                .results
                .first()
                .map(|result| result.path.as_str()),
            Some("current.md"),
            "temporal filtering must run before final limit truncation: {:#?}",
            limited_default.results
        );
        let current = &default.results[0];
        assert_eq!(current.validity.state.as_str(), "current");
        assert!(current.validity.stale_reason.is_none(), "{current:#?}");
        assert!(current.score_breakdown.confidence_boost > 0.0);
        assert!(current.score_breakdown.status_boost > 0.0);
        assert!(current.score_breakdown.evidence_count_boost > 0.0);
        assert!(current.score_breakdown.freshness_boost >= 0.0);

        let include_stale = IndexStore::query_with_options(
            &conn,
            "temporal-quality needle old superseded",
            &QueryOptions {
                limit: 10,
                include_stale: true,
                ..QueryOptions::new(10)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert!(
            include_stale.results.iter().any(|result| {
                result.path == "stale.md"
                    && result.validity.state == "stale"
                    && result.validity.stale_reason.as_deref() == Some("status:stale")
            }),
            "{:#?}",
            include_stale.results
        );

        let historical = IndexStore::query_with_options(
            &conn,
            "temporal-quality needle old superseded",
            &QueryOptions {
                limit: 10,
                as_of: Some("2026-04-15".to_string()),
                freshness: FreshnessMode::Off,
                ..QueryOptions::new(10)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(
            historical
                .results
                .first()
                .map(|result| result.path.as_str()),
            Some("stale.md"),
            "{:#?}",
            historical.results
        );
        assert_eq!(historical.results[0].validity.state.as_str(), "historical");

        let recent = IndexStore::query_with_options(
            &conn,
            "latest temporal-quality needle policy",
            &QueryOptions {
                limit: 10,
                freshness: FreshnessMode::Recent,
                include_stale: true,
                ..QueryOptions::new(10)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(
            recent.results.first().map(|result| result.path.as_str()),
            Some("current.md"),
            "{:#?}",
            recent.results
        );
        assert!(
            recent.results[0].score_breakdown.freshness_boost
                >= recent.results[0].score_breakdown.recency_boost
        );

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_file(&exact_db_path);
    }

    #[test]
    fn query_options_apply_min_score_threshold() {
        let vault = sample_vault();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-threshold-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let baseline = IndexStore::query_with_options(
            &conn,
            "sqlite vec semantic search",
            &QueryOptions::new(10),
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert!(!baseline.results.is_empty());
        let impossible_threshold = baseline.results[0].score + 1.0;

        let filtered = IndexStore::query_with_options(
            &conn,
            "sqlite vec semantic search",
            &QueryOptions {
                limit: 10,
                min_score: Some(impossible_threshold),
                ..QueryOptions::new(10)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        assert!(filtered.results.is_empty(), "{:#?}", filtered.results);
        assert_eq!(filtered.routing.min_score, Some(impossible_threshold));
        assert!(
            filtered.routing.threshold_filtered.unwrap_or_default() >= baseline.results.len(),
            "{:#?}",
            filtered.routing
        );
        assert_eq!(filtered.routing.returned, 0);

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_options_include_neighbor_context_and_obsidian_link_evidence() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-context-links-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("alpha.md"),
            "# Alpha\nBefore context for alpha.\n## Target\nneedle-context-link lives here with [[Bravo]].\n## After\nAfter context for alpha.\n",
        )
        .unwrap();
        fs::write(
            vault.join("bravo.md"),
            "# Bravo\nThis note links back to [[Alpha]] but does not contain the special needle.\n",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-context-links-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let res = IndexStore::query_with_options(
            &conn,
            "needle-context-link",
            &QueryOptions {
                limit: 3,
                context_chunks: 1,
                include_links: true,
                ..QueryOptions::new(3)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let result = res
            .results
            .iter()
            .find(|r| r.path == "alpha.md")
            .expect("alpha result should be present");
        assert!(
            result
                .context_chunks
                .iter()
                .any(|chunk| chunk.relation == "before" && chunk.text.contains("Before context")),
            "{:#?}",
            result.context_chunks
        );
        assert!(
            result
                .context_chunks
                .iter()
                .any(|chunk| chunk.relation == "after" && chunk.text.contains("After context")),
            "{:#?}",
            result.context_chunks
        );
        let links = result
            .evidence
            .links
            .as_ref()
            .expect("link evidence should be included when requested");
        assert!(
            links.outgoing.iter().any(|link| link.target == "Bravo"),
            "{links:#?}"
        );
        assert!(
            links
                .backlinks
                .iter()
                .any(|link| link.source_path == "bravo.md"),
            "{links:#?}"
        );
        assert!(result
            .evidence
            .sources
            .iter()
            .any(|source| source == "wikilink"));
        assert!(result
            .evidence
            .sources
            .iter()
            .any(|source| source == "backlink"));

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_options_expand_obsidian_links_into_candidate_evidence() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-link-expansion-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("alpha.md"),
            "# Alpha\nneedle-hindsight-link-expansion lives here and points to [[Bravo]].\n",
        )
        .unwrap();
        fs::write(
            vault.join("bravo.md"),
            "# Bravo\nThis is the outbound linked note without the special needle.\n",
        )
        .unwrap();
        fs::write(
            vault.join("charlie.md"),
            "# Charlie\nThis backlink note points back to [[Alpha]] without the special needle.\n",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-link-expansion-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let res = IndexStore::query_with_options(
            &conn,
            "needle-hindsight-link-expansion",
            &QueryOptions {
                limit: 6,
                expand_links: 1,
                include_links: true,
                ..QueryOptions::new(6)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        assert_eq!(res.routing.expand_links, 1);
        assert!(res.routing.link_candidates >= 2, "{:#?}", res.routing);
        assert!(res.routing.timings.keyword_ms <= res.took_ms);
        assert!(res.routing.timings.vector_ms <= res.took_ms);
        assert!(res.routing.timings.link_expansion_ms <= res.took_ms);

        for expected_path in ["bravo.md", "charlie.md"] {
            let linked = res
                .results
                .iter()
                .find(|r| r.path == expected_path)
                .unwrap_or_else(|| {
                    panic!(
                        "{expected_path} should be expanded into results: {:#?}",
                        res.results
                    )
                });
            assert!(
                linked
                    .evidence
                    .sources
                    .iter()
                    .any(|source| source == "link_expansion"),
                "{linked:#?}"
            );
            assert!(
                linked.score_breakdown.link_boost > 0.0,
                "{:#?}",
                linked.score_breakdown
            );
            assert!(linked.evidence.evidence_count >= linked.evidence.sources.len());
        }

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_options_retrieval_depth_one_marks_expanded_evidence() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-retrieval-depth-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("alpha.md"),
            "# Alpha\nneedle-graph-depth lives here and points to [[Bravo]].\n",
        )
        .unwrap();
        fs::write(
            vault.join("bravo.md"),
            "# Bravo\nThis linked note has no special needle.\n",
        )
        .unwrap();
        fs::write(
            vault.join("charlie.md"),
            "# Charlie\nThis backlink note points to [[Alpha]] without the special needle.\n",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-retrieval-depth-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let depth_zero = IndexStore::query_with_options(
            &conn,
            "needle-graph-depth",
            &QueryOptions {
                limit: 6,
                retrieval_depth: 0,
                ..QueryOptions::new(6)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(depth_zero.routing.retrieval_depth, 0);
        assert_eq!(depth_zero.routing.link_candidates, 0);
        assert!(depth_zero
            .results
            .iter()
            .all(|r| r.evidence.retrieval_depth == 0));
        assert!(depth_zero.results.iter().all(|r| !r
            .evidence
            .sources
            .iter()
            .any(|s| s == "link_expansion")));

        let depth_one = IndexStore::query_with_options(
            &conn,
            "needle-graph-depth",
            &QueryOptions {
                limit: 6,
                retrieval_depth: 1,
                include_links: true,
                ..QueryOptions::new(6)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert_eq!(depth_one.routing.retrieval_depth, 1);
        assert_eq!(depth_one.routing.expand_links, 1);
        assert!(
            depth_one.routing.link_candidates >= 2,
            "{:#?}",
            depth_one.routing
        );

        assert!(
            depth_one
                .results
                .iter()
                .any(|r| r.path == "alpha.md" && r.evidence.sources.iter().any(|s| s == "keyword")),
            "direct matching result must stay present: {:#?}",
            depth_one.results
        );

        for expected_path in ["bravo.md", "charlie.md"] {
            let linked = depth_one
                .results
                .iter()
                .find(|r| r.path == expected_path)
                .unwrap_or_else(|| {
                    panic!(
                        "{expected_path} should be expanded: {:#?}",
                        depth_one.results
                    )
                });
            assert_eq!(linked.evidence.retrieval_depth, 1, "{linked:#?}");
            assert!(linked
                .evidence
                .sources
                .iter()
                .any(|s| s == "link_expansion"));
            assert!(linked.score_breakdown.link_boost > 0.0);
        }

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn query_options_retrieval_depth_rejects_gt_one() {
        let vault = sample_vault();
        let db_path = std::env::temp_dir().join(format!(
            "orderk-retrieval-depth-invalid-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let err = IndexStore::query_with_options(
            &conn,
            "alpha",
            &QueryOptions {
                limit: 3,
                retrieval_depth: 2,
                ..QueryOptions::new(3)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("--retrieval-depth currently supports 0 or 1"));

        let _ = fs::remove_dir_all(vault);
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn query_options_retrieval_depth_aliases_expand_links() {
        let options = QueryOptions {
            limit: 4,
            expand_links: 1,
            retrieval_depth: 0,
            ..QueryOptions::new(4)
        };
        assert_eq!(options.effective_retrieval_depth().unwrap(), 1);
    }

    #[test]
    fn metadata_boost_positive_for_has_code_negative_for_has_incomplete_tasks() {
        let mut vault = std::env::temp_dir();
        vault.push(format!(
            "orderk-meta-boost-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("code.md"),
            "---\ntags: [meta-test]\n---\n# Code Note\nRust code example:\n```rust\nfn main() {}\n```\n",
        )
        .unwrap();
        fs::write(
            vault.join("task.md"),
            "# Task Note\n- [ ] incomplete task\n",
        )
        .unwrap();
        fs::write(
            vault.join("plain.md"),
            "# Plain Note\nJust text no code no tasks.\n",
        )
        .unwrap();

        let db_path = std::env::temp_dir().join(format!(
            "orderk-meta-boost-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(
            &db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let res = IndexStore::query_with_options(
            &conn,
            "code task",
            &QueryOptions {
                limit: 5,
                ..QueryOptions::new(5)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();

        let code_result = res.results.iter().find(|r| r.path == "code.md");
        let task_result = res.results.iter().find(|r| r.path == "task.md");
        let plain_result = res.results.iter().find(|r| r.path == "plain.md");

        assert!(
            code_result.is_some(),
            "code.md must appear in results for query"
        );
        assert!(
            task_result.is_some(),
            "task.md must appear in results for query"
        );
        assert!(
            plain_result.is_some(),
            "plain.md must appear in results for query"
        );

        // has_code => positive boost
        assert!(
            code_result.unwrap().score_breakdown.metadata_boost > 0.0,
            "code chunk should get positive metadata boost, got {:?}",
            code_result.unwrap().score_breakdown
        );
        // has_incomplete_tasks && no has_code => boost may be zero or slightly negative
        assert!(
            task_result.unwrap().score_breakdown.metadata_boost <= 0.0,
            "incomplete task chunk should get non-positive boost, got {:?}",
            task_result.unwrap().score_breakdown
        );
        // plain text has neither has_code nor has_task_list nor has_link nor has_incomplete_tasks
        assert!(
            (plain_result.unwrap().score_breakdown.metadata_boost - 0.0).abs() < f32::EPSILON,
            "plain chunk should get zero metadata boost, got {:?}",
            plain_result.unwrap().score_breakdown
        );

        let no_rerank = IndexStore::query_with_options(
            &conn,
            "code task",
            &QueryOptions {
                limit: 5,
                rerank: false,
                ..QueryOptions::new(5)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        let code_no_rerank = no_rerank
            .results
            .iter()
            .find(|r| r.path == "code.md")
            .expect("code.md must still appear when rerank is disabled");
        let code_with_rerank = code_result.unwrap();
        assert_eq!(code_no_rerank.score_breakdown.metadata_boost, 0.0);
        assert!(
            (code_with_rerank.score
                - code_no_rerank.score
                - code_with_rerank.score_breakdown.metadata_boost)
                .abs()
                < 0.0001,
            "rerank=false score must remove metadata_boost: with={:?}, without={:?}",
            code_with_rerank,
            code_no_rerank
        );

        let threshold_between_scores = code_no_rerank.score + 0.001;
        let thresholded_without_rerank = IndexStore::query_with_options(
            &conn,
            "code task",
            &QueryOptions {
                limit: 5,
                min_score: Some(threshold_between_scores),
                rerank: false,
                ..QueryOptions::new(5)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert!(
            thresholded_without_rerank
                .results
                .iter()
                .all(|r| r.path != "code.md"),
            "min_score with rerank=false must filter against unboosted score"
        );
        let thresholded_with_rerank = IndexStore::query_with_options(
            &conn,
            "code task",
            &QueryOptions {
                limit: 5,
                min_score: Some(threshold_between_scores),
                ..QueryOptions::new(5)
            },
            &provider,
            &VectorBackend::SqliteVec,
        )
        .unwrap();
        assert!(
            thresholded_with_rerank
                .results
                .iter()
                .any(|r| r.path == "code.md"),
            "default rerank should apply metadata boost before min_score filtering"
        );

        let exact_db_path = std::env::temp_dir().join(format!(
            "orderk-meta-boost-exact-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let mut exact_conn = open_db(
            &exact_db_path,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        IndexStore::index_vault(
            &mut exact_conn,
            &vault,
            &provider,
            provider.dimension(),
            provider.model_id(),
            &VectorBackend::Exact,
        )
        .unwrap();
        let exact_no_rerank = IndexStore::query_with_options(
            &exact_conn,
            "code task",
            &QueryOptions {
                limit: 5,
                rerank: false,
                ..QueryOptions::new(5)
            },
            &provider,
            &VectorBackend::Exact,
        )
        .unwrap();
        let exact_code = exact_no_rerank
            .results
            .iter()
            .find(|r| r.path == "code.md")
            .expect("exact backend should include code.md");
        assert_eq!(exact_code.score_breakdown.metadata_boost, 0.0);

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_file(&exact_db_path);
    }
}
