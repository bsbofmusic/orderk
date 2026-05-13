use crate::chunker::{chunk_document, has_code, has_incomplete_tasks, has_link, has_task_list};
use crate::embedding::{vector_hash, EmbeddingProvider};
use crate::filter::{compile_filter, FilterSql};
use crate::markdown::parse_markdown;
use crate::models::*;
use crate::scanner::scan_vault;
use anyhow::{anyhow, Context, Result};
use chrono::{TimeZone, Utc};
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use sqlite_vec::sqlite3_vec_init;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Once;
use std::time::Instant;

static SQLITE_VEC_REGISTER: Once = Once::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryRoute {
    Semantic,
    Short,
    Path,
    Tag,
}

impl QueryRoute {
    fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Short => "short",
            Self::Path => "path",
            Self::Tag => "tag",
        }
    }
}

#[derive(Debug, Clone)]
struct QueryPlan {
    route: QueryRoute,
    normalized: String,
    terms: Vec<String>,
    patterns: Vec<String>,
}

impl QueryPlan {
    fn analyze(query: &str) -> Self {
        let raw = query.trim().to_lowercase();
        let normalized = normalize_query(query);
        let terms = normalized
            .split_whitespace()
            .filter(|term| !term.is_empty())
            .map(|term| term.to_string())
            .collect::<Vec<_>>();
        let mut patterns = vec![raw.clone(), normalized.clone()];
        patterns.retain(|s| !s.trim().is_empty());
        patterns.sort();
        patterns.dedup();
        let route = if raw.contains('/') || raw.contains(".md") || raw.starts_with("path:") {
            QueryRoute::Path
        } else if raw.contains('#') || raw.starts_with("tag:") {
            QueryRoute::Tag
        } else if terms.len() <= 1 || query.chars().count() <= 12 {
            QueryRoute::Short
        } else {
            QueryRoute::Semantic
        };
        Self {
            route,
            normalized,
            terms,
            patterns,
        }
    }

    fn keyword_query(&self) -> Option<String> {
        if self.terms.is_empty() {
            return None;
        }
        if matches!(self.route, QueryRoute::Short) && self.terms.len() == 1 {
            return Some(format!("{}*", self.terms[0]));
        }
        Some(self.terms.join(" "))
    }

    fn routes_attempted(&self) -> Vec<String> {
        let mut routes = vec!["keyword".to_string(), "vector".to_string()];
        if matches!(self.route, QueryRoute::Path) {
            routes.insert(0, "path".to_string());
        }
        if matches!(self.route, QueryRoute::Tag) {
            routes.insert(0, "tag".to_string());
        }
        if matches!(self.route, QueryRoute::Short) {
            routes.push("path".to_string());
        }
        routes.sort();
        routes.dedup();
        routes
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
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite3_vec_init as *const (),
        )));
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
    upsert_setting(conn, "schema_version", "4")?;
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
    ];
    let mut added_any = false;
    for (column, sql) in migrations {
        if !chunk_column_exists(conn, column)? {
            conn.execute_batch(sql)?;
            added_any = true;
        }
    }
    if added_any || schema_version.as_deref() != Some("4") {
        backfill_chunk_metadata(conn)?;
    }
    upsert_setting(conn, "schema_version", "4")?;
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
                Some((_, ref hash)) if hash != &file.hash => updated += 1,
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
            if !needs_reindex {
                continue;
            }
            let file_summary = reindex_file(
                conn,
                file,
                provider,
                embedding_dim,
                embedding_model,
                vector_backend,
            )?;
            total_chunks += file_summary.chunks;
            embedded += file_summary.embedded;
            reused += file_summary.reused;
        }

        upsert_setting(conn, "embedding_provider", provider.provider_id())?;
        upsert_setting(conn, "embedding_model", embedding_model)?;
        upsert_setting(conn, "embedding_dim", &embedding_dim.to_string())?;
        upsert_setting(conn, "vector_backend", vector_backend.as_str())?;

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
        let query_id = format!(
            "q_{}_{}",
            chrono::Utc::now().timestamp_micros(),
            std::process::id()
        );
        let plan = QueryPlan::analyze(query);
        let filter_text = options
            .filter
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let filter_sql = compile_filter(filter_text.as_deref(), "c")?;
        provider.health()?;
        ensure_has_embeddings(conn)?;
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
        let filtered_candidates = results.len();
        if let Some(min_score) = options.min_score {
            let before_threshold = results.len();
            results.retain(|result| result.score >= min_score);
            routing.threshold_filtered = Some(before_threshold.saturating_sub(results.len()));
        }
        results.truncate(limit);
        if options.context_chunks > 0 || options.include_links {
            enrich_results(
                conn,
                &mut results,
                options.context_chunks,
                options.include_links,
            )?;
        }
        routing.returned = results.len();
        routing.min_score = options.min_score;
        routing.context_chunks = options.context_chunks;
        routing.include_links = options.include_links;
        if filter_text.is_some() {
            routing.filtered_candidates = Some(filtered_candidates);
            routing.filter_mode = Some(match vector_backend {
                VectorBackend::SqliteVec => "sql_pushdown+filtered_exact_vector".to_string(),
                VectorBackend::Exact => "sql_pushdown".to_string(),
            });
        }
        routing.filter = filter_text;
        Ok(QueryResponse {
            query: query.to_string(),
            query_id,
            took_ms: started.elapsed().as_millis(),
            mode: routing.strategy.clone(),
            route: plan.route.as_str().to_string(),
            routing,
            vector_backend: vector_backend.as_str().to_string(),
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

fn reindex_file<P: EmbeddingProvider + ?Sized>(
    conn: &mut Connection,
    file: &ScannedFile,
    provider: &P,
    embedding_dim: usize,
    embedding_model: &str,
    vector_backend: &VectorBackend,
) -> Result<ReindexFileSummary> {
    let body = fs::read_to_string(&file.abs_path)
        .with_context(|| format!("read {}", file.abs_path.display()))?;
    let parsed = parse_markdown(&file.path, &body)?;
    let chunks = chunk_document(&parsed, 1200);
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
    for (chunk, record) in chunks.iter().zip(records.into_iter()) {
        let record =
            record.ok_or_else(|| anyhow!("missing embedding record for chunk {}", chunk.id))?;
        let tags = serde_json::to_string(&chunk.tags)?;
        tx.execute(
            "INSERT INTO chunks(chunk_id, file_path, file_hash, title, heading, line_start, line_end, text, tags_json, links_json, has_code, has_link, has_task_list, has_incomplete_tasks, confidence, status, source_type, chunk_hash, mtime)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
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

    let qvec = provider.embed_query(query)?;
    let vector_scores = if let Some(filter) = filter {
        filtered_exact_vector_scores(conn, &qvec, limit, filter)?
    } else {
        sqlite_vec_vector_scores(conn, &qvec, limit)?
    };

    let route_scores = collect_route_hits(conn, plan, limit, filter)?;
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
            query,
            filter,
            rerank,
        )? {
            results.push(result);
        }
    }
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

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
        keyword_candidates: keyword_scores.len(),
        vector_candidates: vector_scores.len(),
        route_candidates: route_scores.len(),
        merged_candidates,
        returned: 0,
    };
    Ok((results, routing))
}

fn query_exact<P: EmbeddingProvider + ?Sized>(
    conn: &Connection,
    query: &str,
    plan: &QueryPlan,
    limit: usize,
    provider: &P,
    filter: Option<&FilterSql>,
    rerank: bool,
) -> Result<(Vec<SearchResult>, QueryRoutingEvidence)> {
    let qvec = provider.embed_query(query)?;
    let mut sql = String::from(
        "SELECT c.id, c.chunk_id, c.file_path, c.title, c.heading, c.line_start, c.line_end, c.text, c.tags_json, c.mtime, e.embedding, f.mtime, f.hash, c.has_code, c.has_link, c.has_task_list, c.has_incomplete_tasks, c.confidence, c.status, c.source_type
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
        ) = row?;
        let distance = l2_distance(&qvec, &vec);
        let vector_score = distance_to_score(distance);
        let keyword_score = keyword_overlap_score(&plan.normalized, &text, &tags_json);
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
            query,
            has_code,
            has_link,
            has_task_list,
            has_incomplete_tasks,
            confidence,
            status,
            source_type,
            rerank,
        )?;
        scored.push(result);
    }
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
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
    scored.truncate(limit);
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
        keyword_candidates: 0,
        vector_candidates: total_candidates,
        route_candidates: route_matches,
        merged_candidates: total_candidates,
        returned: 0,
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
        "SELECT c.chunk_id, c.file_path, c.title, c.heading, c.line_start, c.line_end, c.text, c.tags_json, c.mtime, f.mtime, c.has_code, c.has_link, c.has_task_list, c.has_incomplete_tasks, c.confidence, c.status, c.source_type
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
        rerank,
    )
    .map(Some)
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
    };
    let score = (keyword_score * 0.35)
        + (vector_score * 0.35)
        + fusion
        + breakdown.path_boost
        + breakdown.tag_boost
        + breakdown.route_boost
        + breakdown.recency_boost
        + metadata_boost;
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
        snippet,
        score,
        score_breakdown: breakdown,
        evidence: SearchResultEvidence {
            sources,
            keyword_rank,
            vector_rank,
            route: Some(plan.route.as_str().to_string()),
            route_score: route_boost,
            links: None,
        },
        context_chunks: Vec::new(),
        tags,
        confidence,
        status,
        source_type,
        mtime: mtime.and_then(|ts| Utc.timestamp_opt(ts, 0).single()),
    })
}

fn normalize_query(query: &str) -> String {
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

fn bm25_to_score(v: f32) -> f32 {
    1.0 / (1.0 + v.abs())
}

fn distance_to_score(v: f32) -> f32 {
    1.0 / (1.0 + v.max(0.0))
}

fn path_boost(path: &str, query: &str) -> f32 {
    let path_l = path.to_lowercase();
    let q = query.to_lowercase();
    if q.split_whitespace().any(|term| path_l.contains(term)) {
        0.08
    } else {
        0.0
    }
}

fn tag_boost(tags: &[String], query: &str) -> f32 {
    let q = query.to_lowercase();
    if tags.iter().any(|t| q.contains(&t.to_lowercase())) {
        0.05
    } else {
        0.0
    }
}

fn recency_boost(mtime: Option<i64>) -> f32 {
    let Some(mtime) = mtime else {
        return 0.0;
    };
    let age_days = ((Utc::now().timestamp() - mtime) as f32 / 86_400.0).max(0.0);
    (1.0 / (1.0 + age_days / 30.0)) * 0.03
}

fn metadata_boost_score(
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

fn normalized_metadata_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn reciprocal_rank_fusion(keyword_rank: Option<usize>, vector_rank: Option<usize>) -> f32 {
    const RRF_K: f32 = 60.0;
    let score = |rank: usize| 1.0 / (RRF_K + rank as f32);
    keyword_rank.map(score).unwrap_or(0.0) + vector_rank.map(score).unwrap_or(0.0)
}

fn snippet(text: &str, query: &str) -> String {
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

fn clean_snippet_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn keyword_overlap_score(query: &str, text: &str, tags_json: &str) -> f32 {
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

fn vector_to_blob(v: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for x in v {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    bytes
}

fn blob_to_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

fn l2_distance(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f32>()
        .sqrt()
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
        assert_eq!(schema_version, "4");
        let _ = fs::remove_file(&db_path);
    }

    #[test]
    fn public_query_migrates_old_schema_before_searching() {
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

        let response = crate::api::query_with_options(
            &db_path,
            "orderk migration",
            &QueryOptions::new(5),
            &provider,
            VectorBackend::Exact,
        )
        .unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].file_path, "legacy.md");
        assert!(
            chunk_column_exists(&rusqlite::Connection::open(&db_path).unwrap(), "confidence")
                .unwrap()
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
