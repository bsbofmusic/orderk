//! Settings KV access and small DB count/existence helpers.
//!
//! Thin wrappers over the `settings` table plus chunk-profile matching and
//! embedding/vector table existence checks. Extracted from `index.rs`.

use crate::models::{default_chunk_max_chars, IndexOptions};
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};
use std::collections::HashMap;

pub(crate) fn bool_to_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

pub(crate) fn upsert_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub(crate) fn setting_value<'a>(
    settings: &'a HashMap<String, String>,
    key: &str,
) -> Option<&'a str> {
    settings.get(key).map(String::as_str)
}

pub(crate) fn chunk_profile_matches(
    settings: &HashMap<String, String>,
    options: &IndexOptions,
) -> bool {
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

pub(crate) fn load_settings_map(conn: &Connection) -> Result<HashMap<String, String>> {
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

pub(crate) fn indexed_embedding_count(conn: &Connection) -> Result<usize> {
    Ok(conn.query_row("SELECT COUNT(*) FROM chunk_embeddings", [], |r| r.get(0))?)
}

pub(crate) fn vec_table_exists(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'vec_chunks'",
        [],
        |r| r.get::<_, usize>(0),
    )
    .unwrap_or(0)
        > 0
}

pub(crate) fn require_setting(
    settings: &HashMap<String, String>,
    key: &str,
    expected: &str,
) -> Result<()> {
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
