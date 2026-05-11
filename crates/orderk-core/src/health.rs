use crate::embedding::EmbeddingProvider;
use crate::index::{register_sqlite_vec, IndexStore};
use crate::models::*;
use rusqlite::Connection;
use serde_json::json;
use std::path::Path;

pub fn classify_error_message(message: &str) -> ErrorCode {
    let lower = message.to_lowercase();
    if lower.contains("unknown embedding provider") {
        ErrorCode::EUnknownProvider
    } else if lower.contains("profile mismatch") || lower.contains("profile missing") {
        ErrorCode::EProfileMismatch
    } else if lower.contains("no embeddings") {
        ErrorCode::ENoEmbeddings
    } else if lower.contains("dimension mismatch") {
        ErrorCode::EEmbeddingDimensionMismatch
    } else if lower.contains("count mismatch") {
        ErrorCode::EEmbeddingCountMismatch
    } else if lower.contains("siliconflow") || lower.contains("api key") || lower.contains("embedding request failed") {
        ErrorCode::EProviderDown
    } else if lower.contains("sqlite-vec") || lower.contains("sqlite_vec") || lower.contains("vec_chunks") {
        ErrorCode::EVectorBackendMissing
    } else if lower.contains("vault") && (lower.contains("read") || lower.contains("unreadable") || lower.contains("missing")) {
        ErrorCode::EVaultUnreadable
    } else if lower.contains("unknown command") || lower.contains("unknown flag") || lower.contains("invalid") || lower.contains("required") {
        ErrorCode::EInvalidArgument
    } else if lower.contains("database disk image") || lower.contains("malformed") || lower.contains("corrupt") {
        ErrorCode::EDbCorrupt
    } else if lower.contains("no such table") || lower.contains("schema") {
        ErrorCode::ESchemaMissing
    } else if lower.contains("sqlite") || lower.contains("db") || lower.contains("database") {
        ErrorCode::EDbOpenFailed
    } else {
        ErrorCode::EInternal
    }
}

#[allow(clippy::too_many_arguments)]
pub fn health_report(
    db_path: &Path,
    vault_path: Option<&Path>,
    provider: Option<&dyn EmbeddingProvider>,
    provider_error: Option<String>,
    expected_provider: &str,
    embedding_dim: usize,
    embedding_model: &str,
    vector_backend: &VectorBackend,
    smoke_query: Option<&str>,
) -> HealthReport {
    let mut checks = Vec::new();

    if let Some(vault) = vault_path {
        if vault.is_dir() {
            checks.push(HealthCheck::ok(
                "vault",
                "vault path is readable",
                json!({"vault": vault.to_string_lossy()}),
            ));
        } else {
            checks.push(HealthCheck::fail(
                "vault",
                ErrorCode::EVaultUnreadable,
                "vault path is missing or not a directory",
                Some("pass a readable Obsidian vault directory via --vault".to_string()),
                json!({"vault": vault.to_string_lossy()}),
            ));
        }
    }

    if !db_path.exists() {
        checks.push(HealthCheck::fail(
            "db",
            ErrorCode::EDbOpenFailed,
            "SQLite database does not exist",
            Some("run `orderk init` or `orderk index` before health/doctor".to_string()),
            json!({"db": db_path.to_string_lossy()}),
        ));
        return finish_report(db_path, vault_path, checks, None);
    }

    register_sqlite_vec();
    let conn = match Connection::open(db_path) {
        Ok(conn) => conn,
        Err(err) => {
            checks.push(HealthCheck::fail(
                "db",
                classify_error_message(&err.to_string()),
                format!("failed to open SQLite database: {err}"),
                Some("verify the DB path and filesystem permissions".to_string()),
                json!({"db": db_path.to_string_lossy()}),
            ));
            return finish_report(db_path, vault_path, checks, None);
        }
    };
    if let Err(err) = conn.busy_timeout(std::time::Duration::from_secs(30)) {
        checks.push(HealthCheck::fail(
            "db",
            classify_error_message(&err.to_string()),
            format!("failed to set SQLite busy timeout: {err}"),
            Some("verify the database is not locked by another process".to_string()),
            json!({"db": db_path.to_string_lossy()}),
        ));
        return finish_report(db_path, vault_path, checks, None);
    }

    let mut status = match IndexStore::status(&conn) {
        Ok(mut status) => {
            status.db = db_path.to_string_lossy().to_string();
            checks.extend(status.checks.clone());
            Some(status)
        }
        Err(err) => {
            checks.push(HealthCheck::fail(
                "schema",
                classify_error_message(&err.to_string()),
                format!("failed to inspect orderk schema: {err}"),
                Some("run `orderk init` or rebuild the orderk SQLite database".to_string()),
                json!({"db": db_path.to_string_lossy()}),
            ));
            return finish_report(db_path, vault_path, checks, None);
        }
    };

    if let Some(err) = provider_error {
        checks.push(HealthCheck::fail(
            "embedding_provider",
            classify_error_message(&err),
            format!("embedding provider `{expected_provider}` is not available: {err}"),
            Some("set the provider credentials/profile explicitly; mock is only for tests".to_string()),
            json!({"expected_provider": expected_provider}),
        ));
    } else if let Some(provider) = provider {
        match provider.health() {
            Ok(()) => checks.push(HealthCheck::ok(
                "embedding_provider",
                "embedding provider health check passed",
                json!({"provider": provider.provider_id(), "model": provider.model_id(), "dim": provider.dimension()}),
            )),
            Err(err) => checks.push(HealthCheck::fail(
                "embedding_provider",
                classify_error_message(&err.to_string()),
                format!("embedding provider health check failed: {err}"),
                Some("verify API key, network, model, and dimension settings".to_string()),
                json!({"provider": provider.provider_id(), "model": provider.model_id(), "dim": provider.dimension()}),
            )),
        }
    }

    if let Some(status_ref) = status.as_ref() {
        if status_ref.embeddings > 0 {
            profile_check(
                &mut checks,
                "embedding_provider",
                &status_ref.embedding_provider,
                expected_provider,
                "provider",
            );
            profile_check(
                &mut checks,
                "embedding_model",
                &status_ref.embedding_model,
                embedding_model,
                "model",
            );
            if status_ref.embedding_dim != embedding_dim {
                checks.push(HealthCheck::fail(
                    "embedding_dim",
                    ErrorCode::EProfileMismatch,
                    format!("embedding dimension mismatch: existing {}, expected {}", status_ref.embedding_dim, embedding_dim),
                    Some("search/index with the same embedding dimension or rebuild the DB".to_string()),
                    json!({"existing": status_ref.embedding_dim, "expected": embedding_dim}),
                ));
            }
            profile_check(
                &mut checks,
                "vector_backend",
                &status_ref.vector_backend,
                vector_backend.as_str(),
                "vector backend",
            );
        }
    }

    if let (Some(provider), Some(query)) = (provider, smoke_query.filter(|q| !q.trim().is_empty())) {
        let blocking_errors = collect_error_codes(&checks);
        let can_search = status.as_ref().map(|s| s.embeddings > 0).unwrap_or(false)
            && !blocking_errors.iter().any(|code| matches!(
                code,
                &ErrorCode::EProfileMismatch
                    | &ErrorCode::EProviderDown
                    | &ErrorCode::EVectorBackendMissing
                    | &ErrorCode::EDbOpenFailed
                    | &ErrorCode::EDbCorrupt
                    | &ErrorCode::ESchemaMissing
            ));
        if can_search {
            match IndexStore::query(&conn, query, 1, provider, vector_backend) {
                Ok(resp) if !resp.results.is_empty() => checks.push(HealthCheck::ok(
                    "smoke_query",
                    "smoke query returned at least one result",
                    json!({"query": query, "query_id": resp.query_id, "results": resp.results.len()}),
                )),
                Ok(resp) => checks.push(HealthCheck::fail(
                    "smoke_query",
                    ErrorCode::ESmokeQueryFailed,
                    "smoke query returned zero results",
                    Some("verify the index contains searchable content or choose a smoke query known to exist".to_string()),
                    json!({"query": query, "query_id": resp.query_id}),
                )),
                Err(err) => checks.push(HealthCheck::fail(
                    "smoke_query",
                    classify_error_message(&err.to_string()),
                    format!("smoke query failed: {err}"),
                    Some("inspect provider/profile/vector backend health before retrying".to_string()),
                    json!({"query": query}),
                )),
            }
        }
    }

    finish_report(db_path, vault_path, checks, status.take())
}

fn profile_check(checks: &mut Vec<HealthCheck>, component: &str, existing: &str, expected: &str, label: &str) {
    if existing != expected {
        checks.push(HealthCheck::fail(
            component,
            ErrorCode::EProfileMismatch,
            format!("{label} mismatch: existing `{existing}`, expected `{expected}`"),
            Some("use matching flags or rebuild the index DB".to_string()),
            json!({"existing": existing, "expected": expected}),
        ));
    }
}

fn collect_error_codes(checks: &[HealthCheck]) -> Vec<ErrorCode> {
    let mut codes = Vec::new();
    for check in checks {
        if let Some(code) = check.error_code.clone() {
            if !codes.contains(&code) {
                codes.push(code);
            }
        }
    }
    codes
}

fn finish_report(
    db_path: &Path,
    vault_path: Option<&Path>,
    checks: Vec<HealthCheck>,
    status: Option<StatusResponse>,
) -> HealthReport {
    let error_codes = collect_error_codes(&checks);
    let state = HealthState::from_error_codes(&error_codes);
    HealthReport {
        schema_version: "orderk.health.v1".to_string(),
        ok: state == HealthState::Ready,
        state,
        db: db_path.to_string_lossy().to_string(),
        vault: vault_path.map(|p| p.to_string_lossy().to_string()),
        checks,
        error_codes,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::MockEmbeddingProvider;
    use crate::index::open_db;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("orderk-{name}-{}-{unique}.sqlite", std::process::id()))
    }

    #[test]
    fn health_reports_missing_db_as_unhealthy() {
        let db = temp_db("missing-health");
        let report = health_report(
            &db,
            None,
            None,
            None,
            "mock",
            8,
            "mock-8",
            &VectorBackend::SqliteVec,
            None,
        );
        assert_eq!(report.state, HealthState::Unhealthy);
        assert!(report.error_codes.contains(&ErrorCode::EDbOpenFailed));
    }

    #[test]
    fn health_reports_initialized_empty_db_as_needs_index() {
        let db = temp_db("empty-health");
        let provider = MockEmbeddingProvider::new(8);
        let _conn = open_db(&db, 8, "mock-8", &VectorBackend::SqliteVec).unwrap();
        let report = health_report(
            &db,
            None,
            Some(&provider),
            None,
            "mock",
            8,
            "mock-8",
            &VectorBackend::SqliteVec,
            None,
        );
        assert_eq!(report.state, HealthState::NeedsIndex);
        assert!(report.error_codes.contains(&ErrorCode::ENoEmbeddings));
        let _ = fs::remove_file(&db);
    }
}
