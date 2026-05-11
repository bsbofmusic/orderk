
use crate::embedding::{vector_hash, EmbeddingProvider};
use crate::models::*;
use crate::markdown::parse_markdown;
use crate::chunker::chunk_document;
use crate::scanner::scan_vault;
use anyhow::{anyhow, Context, Result};
use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use sqlite_vec::sqlite3_vec_init;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Once;
use std::time::Instant;

static SQLITE_VEC_REGISTER: Once = Once::new();

pub(crate) fn register_sqlite_vec() {
    SQLITE_VEC_REGISTER.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ())));
    });
}

pub fn open_db(path: &Path, embedding_dim: usize, embedding_model: &str, vector_backend: &VectorBackend) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    register_sqlite_vec();
    let conn = Connection::open(path).with_context(|| format!("open db {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")?;
    init_schema(&conn, embedding_dim, embedding_model, vector_backend)?;
    Ok(conn)
}

pub fn init_schema(conn: &Connection, embedding_dim: usize, embedding_model: &str, vector_backend: &VectorBackend) -> Result<()> {
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

    ensure_schema_profile(conn, embedding_dim, embedding_model, vector_backend)?;
    let vec_sql = format!("CREATE VIRTUAL TABLE IF NOT EXISTS vec_chunks USING vec0(embedding float[{}])", embedding_dim);
    conn.execute_batch(&vec_sql)?;
    upsert_setting(conn, "embedding_dim", &embedding_dim.to_string())?;
    upsert_setting(conn, "embedding_model", embedding_model)?;
    upsert_setting(conn, "vector_backend", vector_backend.as_str())?;
    upsert_setting(conn, "vector_backend_mode", vector_backend.as_str())?;
    Ok(())
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
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
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
    ).unwrap_or(0) > 0
}

fn ensure_schema_profile(conn: &Connection, embedding_dim: usize, embedding_model: &str, vector_backend: &VectorBackend) -> Result<()> {
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
        return Err(anyhow!("orderk database has no embeddings yet; run `orderk index` before search"));
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
    pub fn open(db_path: &Path, embedding_dim: usize, embedding_model: &str, vector_backend: &VectorBackend, vault_path: &Path) -> Result<Self> {
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
        let embeddings: usize = conn.query_row("SELECT COUNT(*) FROM chunk_embeddings", [], |r| r.get(0))?;
        let vec_version = conn.query_row("SELECT vec_version()", [], |r| r.get::<_, String>(0)).optional().unwrap_or(None);
        let vec_tables: usize = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'vec_chunks'",
            [],
            |r| r.get(0),
        ).unwrap_or(0);
        let embedding_dim = settings.get("embedding_dim").and_then(|v| v.parse().ok()).unwrap_or_default();
        Ok(StatusResponse {
            ok: true,
            db: String::new(),
            notes,
            chunks,
            embeddings,
            fts_enabled: true,
            vector_enabled: vec_version.is_some() && vec_tables == 1,
            vector_backend: settings.get("vector_backend").cloned().unwrap_or_else(|| "unknown".to_string()),
            vec_version,
            embedding_provider: settings.get("embedding_provider").cloned().unwrap_or_else(|| "unknown".to_string()),
            embedding_model: settings.get("embedding_model").cloned().unwrap_or_default(),
            embedding_dim,
        })
    }

    pub fn index_vault<P: EmbeddingProvider + ?Sized>(conn: &mut Connection, vault: &Path, provider: &P, embedding_dim: usize, embedding_model: &str, vector_backend: &VectorBackend) -> Result<IndexSummary> {
        let started = Instant::now();
        init_schema(conn, embedding_dim, embedding_model, vector_backend)?;
        provider.health()?;
        ensure_runtime_profile(conn, provider, embedding_dim, embedding_model, vector_backend)?;
        conn.execute(
            "INSERT INTO settings(key, value) VALUES('embedding_provider', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![provider.provider_id()],
        )?;

        let scanned = scan_vault(vault)?;
        let mut seen_paths = HashSet::new();
        let mut existing: HashMap<String, (i64, String)> = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT path, id, hash FROM files")?;
            let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)))?;
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

        let to_delete: Vec<String> = existing.keys().filter(|path| !seen_paths.contains(*path)).cloned().collect();
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
            reindex_file(conn, &file, provider, embedding_dim, embedding_model, vector_backend)?;
            let chunks_for_file: usize = conn.query_row("SELECT COUNT(*) FROM chunks WHERE file_path = ?1", params![file.path], |r| r.get(0))?;
            total_chunks += chunks_for_file;
            embedded += chunks_for_file;
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
            embedding_provider: provider.provider_id().to_string(),
            embedding_model: provider.model_id().to_string(),
            vector_backend: vector_backend.as_str().to_string(),
            took_ms: started.elapsed().as_millis(),
        })
    }

    pub fn query<P: EmbeddingProvider + ?Sized>(conn: &Connection, query: &str, limit: usize, provider: &P, vector_backend: &VectorBackend) -> Result<QueryResponse> {
        let started = Instant::now();
        let query_id = format!("q_{}_{}", chrono::Utc::now().timestamp_micros(), std::process::id());
        let normalized = normalize_query(query);
        provider.health()?;
        ensure_has_embeddings(conn)?;
        ensure_runtime_profile(conn, provider, provider.dimension(), provider.model_id(), vector_backend)?;
        let mut results = match vector_backend {
            VectorBackend::SqliteVec => query_hybrid(conn, query, &normalized, limit, provider)?,
            VectorBackend::Exact => query_exact(conn, query, &normalized, limit, provider)?,
        };
        results.truncate(limit);
        Ok(QueryResponse {
            query: query.to_string(),
            query_id,
            took_ms: started.elapsed().as_millis(),
            mode: "hybrid".to_string(),
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
        Ok(FeedbackResponse { ok: true, event_id: id as i64 })
    }
}

fn delete_file(conn: &Connection, path: &str) -> Result<()> {
    let mut stmt = conn.prepare("SELECT id FROM chunks WHERE file_path = ?1")?;
    let row_ids = stmt.query_map(params![path], |row| row.get::<_, i64>(0))?;
    let mut ids = Vec::new();
    for id in row_ids { ids.push(id?); }
    for id in &ids {
        conn.execute("DELETE FROM vec_chunks WHERE rowid = ?1", params![id])?;
    }
    conn.execute("DELETE FROM fts_chunks WHERE rowid IN (SELECT id FROM chunks WHERE file_path = ?1)", params![path])?;
    conn.execute("DELETE FROM chunk_embeddings WHERE chunk_id IN (SELECT chunk_id FROM chunks WHERE file_path = ?1)", params![path])?;
    conn.execute("DELETE FROM chunks WHERE file_path = ?1", params![path])?;
    conn.execute("DELETE FROM files WHERE path = ?1", params![path])?;
    Ok(())
}

fn delete_file_in_tx(tx: &rusqlite::Transaction<'_>, path: &str) -> Result<()> {
    let mut stmt = tx.prepare("SELECT id FROM chunks WHERE file_path = ?1")?;
    let row_ids = stmt.query_map(params![path], |row| row.get::<_, i64>(0))?;
    let mut ids = Vec::new();
    for id in row_ids { ids.push(id?); }
    for id in &ids {
        tx.execute("DELETE FROM vec_chunks WHERE rowid = ?1", params![id])?;
    }
    tx.execute("DELETE FROM fts_chunks WHERE rowid IN (SELECT id FROM chunks WHERE file_path = ?1)", params![path])?;
    tx.execute("DELETE FROM chunk_embeddings WHERE chunk_id IN (SELECT chunk_id FROM chunks WHERE file_path = ?1)", params![path])?;
    tx.execute("DELETE FROM chunks WHERE file_path = ?1", params![path])?;
    tx.execute("DELETE FROM files WHERE path = ?1", params![path])?;
    Ok(())
}

fn reindex_file<P: EmbeddingProvider + ?Sized>(conn: &mut Connection, file: &ScannedFile, provider: &P, embedding_dim: usize, embedding_model: &str, vector_backend: &VectorBackend) -> Result<()> {
    let body = fs::read_to_string(&file.abs_path).with_context(|| format!("read {}", file.abs_path.display()))?;
    let parsed = parse_markdown(&file.path, &body)?;
    let chunks = chunk_document(&parsed, 1200);
    let contents: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
    let vectors = provider.embed_documents(&contents)?;
    if vectors.len() != chunks.len() {
        return Err(anyhow!("embedding count mismatch for file {}", file.path));
    }
    let tx = conn.transaction()?;
    delete_file_in_tx(&tx, &file.path)?;
    tx.execute(
        "INSERT INTO files(path, mtime, size, hash, indexed_at) VALUES(?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(path) DO UPDATE SET mtime=excluded.mtime, size=excluded.size, hash=excluded.hash, indexed_at=excluded.indexed_at",
        params![file.path, file.mtime, file.size as i64, file.hash, Utc::now().to_rfc3339()],
    )?;
    for (chunk, vector) in chunks.iter().zip(vectors.iter()) {
        let tags = serde_json::to_string(&chunk.tags)?;
        tx.execute(
            "INSERT INTO chunks(chunk_id, file_path, file_hash, title, heading, line_start, line_end, text, tags_json, chunk_hash, mtime)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![chunk.id, chunk.file_path, file.hash, chunk.title, chunk.heading, chunk.line_start as i64, chunk.line_end as i64, chunk.text, tags, chunk.hash, file.mtime],
        )?;
        let rowid = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO fts_chunks(rowid, chunk_id, file_path, title, heading, text, tags) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![rowid, chunk.id, chunk.file_path, chunk.title, chunk.heading, chunk.text, serde_json::to_string(&chunk.tags)?],
        )?;
        let emb = vector_to_blob(vector);
        tx.execute(
            "INSERT INTO chunk_embeddings(chunk_id, model, dim, embedding, vector_hash) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![chunk.id, embedding_model, embedding_dim as i64, emb, vector_hash(vector)],
        )?;
        tx.execute(
            "INSERT INTO vec_chunks(rowid, embedding) VALUES(?1, ?2)",
            params![rowid, vector_to_blob(vector)],
        )?;
    }
    tx.commit()?;
    let _ = vector_backend;
    Ok(())
}

fn query_hybrid<P: EmbeddingProvider + ?Sized>(conn: &Connection, query: &str, normalized: &str, limit: usize, provider: &P) -> Result<Vec<SearchResult>> {
    let mut keyword_scores: HashMap<i64, (usize, f32)> = HashMap::new();
    let keyword_query = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if !keyword_query.trim().is_empty() {
        let mut stmt = conn.prepare(
            "SELECT rowid, bm25(fts_chunks) AS score FROM fts_chunks WHERE fts_chunks MATCH ?1 ORDER BY score ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![keyword_query, (limit * 4) as i64], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f32>(1)?)))?;
        for (rank, row) in rows.enumerate() {
            let (rowid, bm25) = row?;
            keyword_scores.insert(rowid, (rank + 1, bm25_to_score(bm25)));
        }
    }

    let qvec = provider.embed_query(query)?;
    let mut vector_scores: HashMap<i64, (usize, f32)> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT rowid, distance FROM vec_chunks WHERE embedding MATCH ?1 AND k = ?2 ORDER BY distance ASC",
    )?;
    let rows = stmt.query_map(params![vector_to_blob(&qvec), (limit * 4) as i64], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f32>(1)?)))?;
    for (rank, row) in rows.enumerate() {
        let (rowid, distance) = row?;
        vector_scores.insert(rowid, (rank + 1, distance_to_score(distance)));
    }

    let mut candidate_ids: HashSet<i64> = keyword_scores.keys().copied().collect();
    candidate_ids.extend(vector_scores.keys().copied());
    let mut results = Vec::new();
    for rowid in candidate_ids {
        if let Some(result) = load_chunk_result(conn, rowid, keyword_scores.get(&rowid), vector_scores.get(&rowid), query)? {
            results.push(result);
        }
    }
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    Ok(results)
}

fn query_exact<P: EmbeddingProvider + ?Sized>(conn: &Connection, query: &str, normalized: &str, limit: usize, provider: &P) -> Result<Vec<SearchResult>> {
    let qvec = provider.embed_query(query)?;
    let mut stmt = conn.prepare(
        "SELECT c.id, c.chunk_id, c.file_path, c.title, c.heading, c.line_start, c.line_end, c.text, c.tags_json, c.mtime, e.embedding, f.mtime, f.hash
         FROM chunks c
         JOIN chunk_embeddings e ON e.chunk_id = c.chunk_id
         LEFT JOIN files f ON f.path = c.file_path",
    )?;
    let rows = stmt.query_map([], |row| {
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
        ))
    })?;
    let mut scored = Vec::new();
    for row in rows {
        let (rowid, chunk_id, path, title, heading, line_start, line_end, text, tags_json, mtime, vec) = row?;
        let distance = l2_distance(&qvec, &vec);
        let vector_score = distance_to_score(distance);
        let keyword_score = keyword_overlap_score(normalized, &text, &tags_json);
        let result = build_result(rowid, &chunk_id, &path, title, heading, line_start, line_end, &text, &tags_json, Some(mtime), None, None, keyword_score, vector_score, query)?;
        scored.push(result);
    }
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    Ok(scored)
}

fn load_chunk_result(
    conn: &Connection,
    rowid: i64,
    keyword: Option<&(usize, f32)>,
    vector: Option<&(usize, f32)>,
    query: &str,
) -> Result<Option<SearchResult>> {
    let mut stmt = conn.prepare(
        "SELECT c.chunk_id, c.file_path, c.title, c.heading, c.line_start, c.line_end, c.text, c.tags_json, c.mtime, f.mtime
         FROM chunks c
         LEFT JOIN files f ON f.path = c.file_path
         WHERE c.id = ?1",
    )?;
    let row = stmt.query_row(params![rowid], |row| {
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
        ))
    }).optional()?;
    let Some((chunk_id, path, title, heading, line_start, line_end, text, tags_json, mtime, file_mtime)) = row else {
        return Ok(None);
    };
    build_result(rowid, &chunk_id, &path, title, heading, line_start, line_end, &text, &tags_json, file_mtime.or(Some(mtime)), keyword.map(|v| v.0), vector.map(|v| v.0), keyword.map(|v| v.1).unwrap_or(0.0), vector.map(|v| v.1).unwrap_or(0.0), query).map(Some)
}

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
    query: &str,
) -> Result<SearchResult> {
    let tags: Vec<String> = serde_json::from_str(tags_json).unwrap_or_default();
    let fusion = reciprocal_rank_fusion(keyword_rank, vector_rank);
    let breakdown = ScoreBreakdown {
        keyword: keyword_score,
        vector: vector_score,
        fusion,
        path_boost: path_boost(path, query),
        tag_boost: tag_boost(&tags, query),
        recency_boost: recency_boost(mtime),
    };
    let score = (keyword_score * 0.35) + (vector_score * 0.35) + fusion + breakdown.path_boost + breakdown.tag_boost + breakdown.recency_boost;
    let snippet = snippet(text, query);
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
        tags,
        mtime: mtime.and_then(|ts| Utc.timestamp_opt(ts, 0).single()),
    })
}

fn normalize_query(query: &str) -> String {
    query.chars().map(|c| if c.is_alphanumeric() || c.is_whitespace() { c } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
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
    if q.split_whitespace().any(|term| path_l.contains(term)) { 0.08 } else { 0.0 }
}

fn tag_boost(tags: &[String], query: &str) -> f32 {
    let q = query.to_lowercase();
    if tags.iter().any(|t| q.contains(&t.to_lowercase())) { 0.05 } else { 0.0 }
}

fn recency_boost(mtime: Option<i64>) -> f32 {
    let Some(mtime) = mtime else { return 0.0; };
    let age_days = ((Utc::now().timestamp() - mtime) as f32 / 86_400.0).max(0.0);
    (1.0 / (1.0 + age_days / 30.0)) * 0.03
}

fn reciprocal_rank_fusion(keyword_rank: Option<usize>, vector_rank: Option<usize>) -> f32 {
    const RRF_K: f32 = 60.0;
    let score = |rank: usize| 1.0 / (RRF_K + rank as f32);
    keyword_rank.map(score).unwrap_or(0.0) + vector_rank.map(score).unwrap_or(0.0)
}

fn snippet(text: &str, query: &str) -> String {
    let lower = text.to_lowercase();
    for term in query.split_whitespace() {
        if term.is_empty() { continue; }
        if let Some(pos) = lower.find(&term.to_lowercase()) {
            let start = pos.saturating_sub(60);
            let end = (pos + term.len() + 120).min(text.len());
            return text[start..end].trim().replace('\n', " ");
        }
    }
    text.chars().take(180).collect::<String>().replace('\n', " ")
}

fn keyword_overlap_score(query: &str, text: &str, tags_json: &str) -> f32 {
    let tags: Vec<String> = serde_json::from_str(tags_json).unwrap_or_default();
    let q = query.to_lowercase();
    let mut score: f32 = 0.0;
    for term in q.split_whitespace() {
        if text.to_lowercase().contains(term) { score += 0.1; }
        if tags.iter().any(|t| t.to_lowercase().contains(term)) { score += 0.05; }
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
    bytes.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

fn l2_distance(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| (x - y) * (x - y)).sum::<f32>().sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::MockEmbeddingProvider;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample_vault() -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        dir.push(format!("orderk-index-{}-{}", std::process::id(), unique));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("alpha.md"), "---\ntags: [project, alpha]\n---\n# Alpha Project\nThe alpha project uses sqlite-vec local semantic search.\nIt includes chunking and FTS.\n").unwrap();
        fs::write(dir.join("bravo.md"), "# Bravo Project\nObsidian plugin packaging and npm workspace builds.").unwrap();
        dir
    }

    #[test]
    fn index_and_query_roundtrip() {
        let vault = sample_vault();
        let db_path = std::env::temp_dir().join(format!("orderk-db-{}-{}.sqlite", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(&db_path, provider.dimension(), provider.model_id(), &VectorBackend::SqliteVec).unwrap();
        let summary = IndexStore::index_vault(&mut conn, &vault, &provider, provider.dimension(), provider.model_id(), &VectorBackend::SqliteVec).unwrap();
        assert_eq!(summary.files, 2);
        let res = IndexStore::query(&conn, "sqlite vec semantic search", 5, &provider, &VectorBackend::SqliteVec).unwrap();
        assert!(!res.results.is_empty());
        assert_eq!(res.results[0].path, "alpha.md");
        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
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
        vault.push(format!("orderk-failed-reindex-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()));
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("alpha.md"), "# Alpha\nDurable sqlite vec semantic search should survive failed refresh.").unwrap();

        let db_path = std::env::temp_dir().join(format!("orderk-failed-reindex-{}-{}.sqlite", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()));
        let provider = MockEmbeddingProvider::new(8);
        let mut conn = open_db(&db_path, provider.dimension(), provider.model_id(), &VectorBackend::SqliteVec).unwrap();
        IndexStore::index_vault(&mut conn, &vault, &provider, provider.dimension(), provider.model_id(), &VectorBackend::SqliteVec).unwrap();
        let before = IndexStore::status(&conn).unwrap();
        assert_eq!(before.notes, 1);
        assert_eq!(before.chunks, 1);
        assert_eq!(before.embeddings, 1);

        fs::write(vault.join("alpha.md"), "# Alpha\nChanged content that would require a fresh embedding.").unwrap();
        let err = IndexStore::index_vault(&mut conn, &vault, &FailingSameProfileProvider, 8, "mock-8", &VectorBackend::SqliteVec).unwrap_err();
        assert!(err.to_string().contains("forced embedding failure"), "{err:?}");

        let after = IndexStore::status(&conn).unwrap();
        assert_eq!(after.notes, before.notes);
        assert_eq!(after.chunks, before.chunks);
        assert_eq!(after.embeddings, before.embeddings);
        let res = IndexStore::query(&conn, "durable sqlite vec semantic search", 5, &provider, &VectorBackend::SqliteVec).unwrap();
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
            Ok(inputs.iter().map(|input| {
                let lower = input.to_lowercase();
                if lower.contains("vector-target-signature") || lower.contains("needle-nearest-neighbor") {
                    vec![1.0, 0.0, 0.0, 0.0]
                } else if lower.contains("vector-decoy-signature") {
                    vec![0.0, 1.0, 0.0, 0.0]
                } else {
                    vec![0.0, 0.0, 1.0, 0.0]
                }
            }).collect())
        }
    }

    #[test]
    fn sqlite_vec_backend_returns_vector_only_match_without_keyword_hit() {
        let mut vault = std::env::temp_dir();
        vault.push(format!("orderk-vector-only-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()));
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("target.md"), "# Target\nvector-target-signature lives in this unrelated note.").unwrap();
        fs::write(vault.join("decoy.md"), "# Decoy\nvector-decoy-signature lives in another unrelated note.").unwrap();

        let db_path = std::env::temp_dir().join(format!("orderk-vector-only-{}-{}.sqlite", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()));
        let provider = ControlledVectorProvider;
        let mut conn = open_db(&db_path, provider.dimension(), provider.model_id(), &VectorBackend::SqliteVec).unwrap();
        let status = IndexStore::status(&conn).unwrap();
        assert!(status.vector_enabled, "sqlite-vec extension/table must be enabled");

        IndexStore::index_vault(&mut conn, &vault, &provider, provider.dimension(), provider.model_id(), &VectorBackend::SqliteVec).unwrap();
        let res = IndexStore::query(&conn, "needle-nearest-neighbor", 2, &provider, &VectorBackend::SqliteVec).unwrap();
        assert_eq!(res.vector_backend, "sqlite_vec");
        assert_eq!(res.results[0].path, "target.md");
        assert_eq!(res.results[0].score_breakdown.keyword, 0.0);
        assert!(res.results[0].score_breakdown.vector > res.results[1].score_breakdown.vector);

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_file(&db_path);
    }
}
