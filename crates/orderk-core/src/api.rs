use crate::embedding::{
    EmbeddingProvider, MockEmbeddingProvider, OpenAiCompatibleEmbeddingConfig,
    OpenAiCompatibleEmbeddingProvider, SiliconFlowM3Provider,
};
use crate::index::{open_db, IndexStore};
use crate::models::*;
use crate::optimizer;
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
    index_vault_with_options(
        vault,
        db_path,
        provider,
        embedding_dim,
        embedding_model,
        vector_backend,
        &IndexOptions::default(),
    )
}

pub fn index_vault_with_options(
    vault: &Path,
    db_path: &Path,
    provider: &dyn EmbeddingProvider,
    embedding_dim: usize,
    embedding_model: &str,
    vector_backend: VectorBackend,
    options: &IndexOptions,
) -> Result<IndexSummary> {
    let mut store = IndexStore::open(
        db_path,
        embedding_dim,
        embedding_model,
        &vector_backend,
        vault,
    )?;
    let summary = IndexStore::index_vault_with_options(
        &mut store.conn,
        vault,
        provider,
        embedding_dim,
        embedding_model,
        &vector_backend,
        options,
    )?;
    Ok(IndexSummary {
        db: db_path.to_string_lossy().to_string(),
        ..summary
    })
}

pub fn index_paths_with_options(
    vault: &Path,
    db_path: &Path,
    provider: &dyn EmbeddingProvider,
    embedding_dim: usize,
    embedding_model: &str,
    vector_backend: VectorBackend,
    options: &IndexPathOptions,
) -> Result<IndexSummary> {
    let mut store = IndexStore::open(
        db_path,
        embedding_dim,
        embedding_model,
        &vector_backend,
        vault,
    )?;
    let summary = IndexStore::index_paths_with_options(
        &mut store.conn,
        vault,
        provider,
        embedding_dim,
        embedding_model,
        &vector_backend,
        options,
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
    let read_conn = open_existing(db_path)?;
    let mut response =
        IndexStore::query_with_options(&read_conn, query, options, provider, &vector_backend)?;
    response.optimizer = Some(optimizer::with_model_hint(
        optimizer::optimizer_status(&read_conn)
            .unwrap_or_else(|_| optimizer::disabled_optimizer_status()),
        provider.model_id(),
    ));
    Ok(response)
}

pub fn optimize_status(db_path: &Path) -> Result<OptimizerStatus> {
    let conn = open_existing(db_path)?;
    optimizer::optimizer_status(&conn)
}

pub fn optimize_dry_run(db_path: &Path, min_events: usize) -> Result<OptimizeResponse> {
    let conn = open_existing(db_path)?;
    optimizer::dry_run_optimizer(&conn, min_events)
}

pub fn optimize_apply(db_path: &Path, min_events: usize) -> Result<OptimizeResponse> {
    let conn = open_writable_existing(db_path)?;
    optimizer::apply_optimizer(&conn, min_events)
}

pub fn optimize_reset(db_path: &Path) -> Result<OptimizeResponse> {
    let conn = open_writable_existing(db_path)?;
    optimizer::reset_optimizer(&conn)
}

pub fn optimize_set(
    db_path: &Path,
    text_only_penalty: Option<f64>,
    add_stopwords: &[String],
    remove_stopwords: &[String],
) -> Result<OptimizeResponse> {
    let conn = open_writable_existing(db_path)?;
    optimizer::set_optimizer(&conn, text_only_penalty, add_stopwords, remove_stopwords)
}

pub fn get_chunks(db_path: &Path, options: &ChunkGetOptions) -> Result<ChunkGetResponse> {
    let conn = open_existing(db_path)?;
    IndexStore::get_chunks(&conn, options)
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
    let name = env_string("ORDERK_SWORD_EMBEDDING_PROVIDER")
        .or_else(|| env_string("ORDERK_EMBEDDING_PROVIDER"))
        .unwrap_or_else(|| "siliconflow".to_string());
    let normalized = normalize_provider_name(&name);
    let model = model
        .or_else(|| env_string(&vendor_env_name(&normalized, "MODEL")))
        .or_else(|| env_string("ORDERK_SWORD_EMBEDDING_MODEL"))
        .or_else(|| env_string("ORDERK_EMBEDDING_MODEL"));
    provider_from_name(&name, dim, model)
}

pub fn provider_from_name(
    name: &str,
    dim: usize,
    model: Option<String>,
) -> Result<Box<dyn EmbeddingProvider>> {
    let normalized = normalize_provider_name(name);
    match normalized.as_str() {
        "mock" => Ok(Box::new(MockEmbeddingProvider::new(dim))),
        "siliconflow" => {
            let key = required_env_any(
                &[
                    "ORDERK_SWORD_EMBEDDING_SILICONFLOW_API_KEY",
                    "ORDERK_SILICONFLOW_API_KEY",
                ],
                "SiliconFlow",
            )?;
            Ok(Box::new(SiliconFlowM3Provider::new(
                key,
                model,
                dim,
                env_string("ORDERK_SWORD_EMBEDDING_SILICONFLOW_BASE_URL")
                    .or_else(|| env_string("ORDERK_SILICONFLOW_BASE_URL")),
            )))
        }
        "openai" => Ok(Box::new(OpenAiCompatibleEmbeddingProvider::new(
            OpenAiCompatibleEmbeddingConfig {
                provider_id: "openai".to_string(),
                label: "OpenAI".to_string(),
                api_key: required_env_any(
                    &[
                        "ORDERK_SWORD_EMBEDDING_OPENAI_API_KEY",
                        "ORDERK_OPENAI_API_KEY",
                        "ORDERK_EMBEDDING_API_KEY",
                    ],
                    "OpenAI",
                )?,
                key_hint: "ORDERK_SWORD_EMBEDDING_OPENAI_API_KEY or ORDERK_OPENAI_API_KEY or ORDERK_EMBEDDING_API_KEY".to_string(),
                model,
                default_model: "text-embedding-3-small".to_string(),
                dim,
                base_url: env_string("ORDERK_SWORD_EMBEDDING_OPENAI_BASE_URL")
                    .or_else(|| env_string("ORDERK_OPENAI_BASE_URL")),
                default_base_url: "https://api.openai.com/v1/embeddings".to_string(),
            },
        ))),
        "openai-compatible" | "generic" => Ok(Box::new(OpenAiCompatibleEmbeddingProvider::new(
            OpenAiCompatibleEmbeddingConfig {
                provider_id: normalized.clone(),
                label: "OpenAI-compatible".to_string(),
                api_key: required_env_any(
                    &[
                        "ORDERK_SWORD_EMBEDDING_OPENAI_COMPATIBLE_API_KEY",
                        "ORDERK_SWORD_EMBEDDING_GENERIC_API_KEY",
                        "ORDERK_SWORD_EMBEDDING_API_KEY",
                        "ORDERK_EMBEDDING_API_KEY",
                    ],
                    "OpenAI-compatible",
                )?,
                key_hint: "ORDERK_SWORD_EMBEDDING_OPENAI_COMPATIBLE_API_KEY or ORDERK_SWORD_EMBEDDING_GENERIC_API_KEY or ORDERK_SWORD_EMBEDDING_API_KEY or ORDERK_EMBEDDING_API_KEY".to_string(),
                model,
                default_model: "text-embedding-3-small".to_string(),
                dim,
                base_url: Some(required_env_any(
                    &[
                        "ORDERK_SWORD_EMBEDDING_OPENAI_COMPATIBLE_BASE_URL",
                        "ORDERK_SWORD_EMBEDDING_GENERIC_BASE_URL",
                        "ORDERK_SWORD_EMBEDDING_BASE_URL",
                        "ORDERK_EMBEDDING_BASE_URL",
                    ],
                    "OpenAI-compatible embedding base URL",
                )?),
                default_base_url: String::new(),
            },
        ))),
        other => Err(anyhow::anyhow!("unknown embedding provider: {}", other)),
    }
}

fn normalize_provider_name(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace('_', "-")
}

fn vendor_env_name(provider: &str, suffix: &str) -> String {
    let vendor = provider.trim().to_ascii_uppercase().replace('-', "_");
    format!("ORDERK_SWORD_EMBEDDING_{vendor}_{suffix}")
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn required_env_any(names: &[&str], label: &str) -> Result<String> {
    for name in names {
        if let Some(value) = env_string(name) {
            return Ok(value);
        }
    }
    Err(anyhow::anyhow!(
        "{label} embedding API key is missing; set {}",
        names.join(" or ")
    ))
}

fn open_existing(db_path: &Path) -> Result<Connection> {
    crate::index::register_sqlite_vec();
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    Ok(conn)
}

fn open_writable_existing(db_path: &Path) -> Result<Connection> {
    crate::index::register_sqlite_vec();
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    crate::index::migrate_chunk_metadata_columns(&conn)?;
    Ok(conn)
}

#[cfg(test)]
mod provider_resolution_tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    const PROVIDER_ENV_NAMES: &[&str] = &[
        "ORDERK_SWORD_EMBEDDING_PROVIDER",
        "ORDERK_SWORD_EMBEDDING_MODEL",
        "ORDERK_SWORD_EMBEDDING_DIM",
        "ORDERK_SWORD_EMBEDDING_API_KEY",
        "ORDERK_SWORD_EMBEDDING_BASE_URL",
        "ORDERK_SWORD_EMBEDDING_SILICONFLOW_MODEL",
        "ORDERK_SWORD_EMBEDDING_SILICONFLOW_DIM",
        "ORDERK_SWORD_EMBEDDING_SILICONFLOW_API_KEY",
        "ORDERK_SWORD_EMBEDDING_SILICONFLOW_BASE_URL",
        "ORDERK_SWORD_EMBEDDING_OPENAI_MODEL",
        "ORDERK_SWORD_EMBEDDING_OPENAI_DIM",
        "ORDERK_SWORD_EMBEDDING_OPENAI_API_KEY",
        "ORDERK_SWORD_EMBEDDING_OPENAI_BASE_URL",
        "ORDERK_SWORD_EMBEDDING_OPENAI_COMPATIBLE_API_KEY",
        "ORDERK_SWORD_EMBEDDING_OPENAI_COMPATIBLE_BASE_URL",
        "ORDERK_SWORD_EMBEDDING_GENERIC_API_KEY",
        "ORDERK_SWORD_EMBEDDING_GENERIC_BASE_URL",
        "ORDERK_EMBEDDING_PROVIDER",
        "ORDERK_EMBEDDING_API_KEY",
        "ORDERK_EMBEDDING_BASE_URL",
        "ORDERK_SILICONFLOW_API_KEY",
        "ORDERK_SILICONFLOW_BASE_URL",
        "ORDERK_OPENAI_API_KEY",
        "ORDERK_OPENAI_BASE_URL",
        "HERMES_SILICONFLOW_API_KEY",
        "HERMES_SF_API_KEY",
        "HERMES_ORDERK_SILICONFLOW_API_KEY",
        "SILICONFLOW_API_KEY",
    ];

    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn with_clean_provider_env<T>(f: impl FnOnce() -> T) -> T {
        let _guard = env_lock();
        let saved = PROVIDER_ENV_NAMES
            .iter()
            .map(|name| (*name, std::env::var(name).ok()))
            .collect::<Vec<_>>();
        for name in PROVIDER_ENV_NAMES {
            std::env::remove_var(name);
        }
        let result = f();
        for (name, value) in saved {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
        result
    }

    #[test]
    fn provider_from_env_honors_orderk_embedding_provider_without_key() {
        with_clean_provider_env(|| {
            std::env::set_var("ORDERK_EMBEDDING_PROVIDER", "mock");
            let provider = provider_from_env(7, None).expect("mock provider should not need a key");
            assert_eq!(provider.provider_id(), "mock");
            assert_eq!(provider.dimension(), 7);
        });
    }

    #[test]
    fn siliconflow_provider_rejects_hermes_and_bare_siliconflow_keys() {
        with_clean_provider_env(|| {
            std::env::set_var("HERMES_SILICONFLOW_API_KEY", "hermes-chat-key");
            std::env::set_var("HERMES_SF_API_KEY", "hermes-provider-key");
            std::env::set_var("SILICONFLOW_API_KEY", "ambiguous-legacy-key");
            let err = match provider_from_name("siliconflow", 3, Some("fixture-model".to_string()))
            {
                Ok(_) => {
                    panic!("siliconflow provider must not accept Hermes or ambiguous legacy keys")
                }
                Err(err) => err.to_string(),
            };
            assert!(err.contains("ORDERK_SILICONFLOW_API_KEY"), "{err}");
            assert!(!err.contains("HERMES"), "{err}");
            assert!(!err.contains("HERMES"), "{err}");
            assert!(!err.contains("ambiguous-legacy-key"), "{err}");
        });
    }

    #[test]
    fn provider_from_env_honors_sword_vendor_specific_model_and_key() {
        with_clean_provider_env(|| {
            std::env::set_var("ORDERK_SWORD_EMBEDDING_PROVIDER", "openai");
            std::env::set_var(
                "ORDERK_SWORD_EMBEDDING_OPENAI_MODEL",
                "fixture-openai-model",
            );
            std::env::set_var(
                "ORDERK_SWORD_EMBEDDING_OPENAI_API_KEY",
                "orderk-sword-openai-key",
            );
            std::env::set_var(
                "ORDERK_SWORD_EMBEDDING_OPENAI_BASE_URL",
                "http://127.0.0.1:1/v1/embeddings",
            );
            let provider = provider_from_env(17, None)
                .expect("provider_from_env should use SWORD vendor-specific env");
            assert_eq!(provider.provider_id(), "openai");
            assert_eq!(provider.model_id(), "fixture-openai-model");
            assert_eq!(provider.dimension(), 17);
        });
    }

    #[test]
    fn openai_compatible_provider_uses_sword_generic_key_and_base_url() {
        with_clean_provider_env(|| {
            std::env::set_var(
                "ORDERK_SWORD_EMBEDDING_OPENAI_COMPATIBLE_API_KEY",
                "orderk-sword-generic-key",
            );
            std::env::set_var(
                "ORDERK_SWORD_EMBEDDING_OPENAI_COMPATIBLE_BASE_URL",
                "http://127.0.0.1:1/v1/embeddings",
            );
            let provider = provider_from_name(
                "openai-compatible",
                6,
                Some("fixture-online-model".to_string()),
            )
            .expect("openai-compatible provider should use SWORD-scoped key/base_url");
            assert_eq!(provider.provider_id(), "openai-compatible");
            assert_eq!(provider.model_id(), "fixture-online-model");
            assert_eq!(provider.dimension(), 6);
        });
    }

    #[test]
    fn openai_compatible_provider_uses_orderk_scoped_generic_key_and_base_url() {
        with_clean_provider_env(|| {
            std::env::set_var("ORDERK_EMBEDDING_API_KEY", "orderk-generic-key");
            std::env::set_var(
                "ORDERK_EMBEDDING_BASE_URL",
                "http://127.0.0.1:1/v1/embeddings",
            );
            let provider = provider_from_name(
                "openai-compatible",
                5,
                Some("fixture-online-model".to_string()),
            )
            .expect("openai-compatible provider should use orderk-scoped key/base_url");
            assert_eq!(provider.provider_id(), "openai-compatible");
            assert_eq!(provider.model_id(), "fixture-online-model");
            assert_eq!(provider.dimension(), 5);
        });
    }
}
