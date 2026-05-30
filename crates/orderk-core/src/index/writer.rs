//! Index write path: file (re)indexing, deletion, and embedding reuse.
//!
//! The DB-mutating core: deleting a file's chunks/embeddings (standalone and
//! in-transaction), reindexing a scanned file (chunk, embed-or-reuse, insert
//! into chunks/fts/embeddings/vec tables), loading reusable embeddings by
//! content hash, and building chunk embedding input text. Extracted from
//! `index.rs`.

use super::links::extract_wikilinks_from_text;
use super::scoring::vector_to_blob;
use super::settings::bool_to_i64;
use crate::chunker::{chunk_document_with_options, ChunkingOptions};
use crate::embedding::{vector_hash, EmbeddingProvider};
use crate::markdown::parse_markdown;
use crate::models::*;
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Clone)]
pub(crate) struct EmbeddingRecord {
    pub(crate) blob: Vec<u8>,
    pub(crate) vector_hash: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ReindexFileSummary {
    pub(crate) chunks: usize,
    pub(crate) embedded: usize,
    pub(crate) reused: usize,
}

pub(crate) fn delete_file(conn: &Connection, path: &str) -> Result<()> {
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

pub(crate) fn delete_file_in_tx(tx: &rusqlite::Transaction<'_>, path: &str) -> Result<()> {
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

pub(crate) fn reindex_file_with_options<P: EmbeddingProvider + ?Sized>(
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

pub(crate) fn load_reusable_embeddings(
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

pub(crate) fn chunk_embedding_input(chunk: &Chunk) -> String {
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
