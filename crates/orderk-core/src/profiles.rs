use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const DEFAULT_EMBEDDING_PROVIDER: &str = "siliconflow";
const DEFAULT_EMBEDDING_MODEL: &str = "Qwen/Qwen3-Embedding-4B";
const DEFAULT_EMBEDDING_DIM: usize = 1024;
const DEFAULT_RERANKER_PROVIDER: &str = "siliconflow";
const DEFAULT_RERANKER_MODEL: &str = "Qwen/Qwen3-Reranker-4B";
const DEFAULT_LLM_PROVIDER: &str = "anthropic";
const DEFAULT_LLM_MODEL: &str = "MiniMax-M3";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwordModelKind {
    Embedding,
    Reranker,
    Llm,
}

impl SwordModelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SwordModelKind::Embedding => "embedding",
            SwordModelKind::Reranker => "reranker",
            SwordModelKind::Llm => "llm",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwordModelSlot {
    pub kind: SwordModelKind,
    pub provider: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dim: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    pub api_key_configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub profile_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwordModelProfile {
    pub schema_version: String,
    pub embedding: SwordModelSlot,
    pub reranker: SwordModelSlot,
    pub llm: SwordModelSlot,
}

pub fn resolve_sword_model_profile_from_env() -> Result<SwordModelProfile> {
    Ok(SwordModelProfile {
        schema_version: "orderk.v2.model_profile.v1".to_string(),
        embedding: resolve_sword_model_slot_from_env(SwordModelKind::Embedding)?,
        reranker: resolve_sword_model_slot_from_env(SwordModelKind::Reranker)?,
        llm: resolve_sword_model_slot_from_env(SwordModelKind::Llm)?,
    })
}

pub fn resolve_sword_model_slot_from_env(kind: SwordModelKind) -> Result<SwordModelSlot> {
    match kind {
        SwordModelKind::Embedding => resolve_embedding_slot(),
        SwordModelKind::Reranker => resolve_reranker_slot(),
        SwordModelKind::Llm => resolve_llm_slot(),
    }
}

fn resolve_embedding_slot() -> Result<SwordModelSlot> {
    let provider = normalize_provider(
        env_string("ORDERK_SWORD_EMBEDDING_PROVIDER")
            .or_else(|| env_string("ORDERK_EMBEDDING_PROVIDER"))
            .unwrap_or_else(|| DEFAULT_EMBEDDING_PROVIDER.to_string()),
    );
    let (model, dim, api_key_env, base_url) = match provider.as_str() {
        "siliconflow" => (
            env_string("ORDERK_SWORD_EMBEDDING_SILICONFLOW_MODEL")
                .or_else(|| env_string("ORDERK_SWORD_EMBEDDING_MODEL"))
                .or_else(|| env_string("ORDERK_EMBEDDING_MODEL"))
                .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_string()),
            env_usize("ORDERK_SWORD_EMBEDDING_SILICONFLOW_DIM")
                .or_else(|| env_usize("ORDERK_SWORD_EMBEDDING_DIM"))
                .or_else(|| env_usize("ORDERK_EMBEDDING_DIM"))
                .unwrap_or(DEFAULT_EMBEDDING_DIM),
            first_configured_env(&[
                "ORDERK_SWORD_EMBEDDING_SILICONFLOW_API_KEY",
                "ORDERK_SILICONFLOW_API_KEY",
            ]),
            env_string("ORDERK_SWORD_EMBEDDING_SILICONFLOW_BASE_URL")
                .or_else(|| env_string("ORDERK_SILICONFLOW_BASE_URL")),
        ),
        "openai" => (
            env_string("ORDERK_SWORD_EMBEDDING_OPENAI_MODEL")
                .or_else(|| env_string("ORDERK_SWORD_EMBEDDING_MODEL"))
                .or_else(|| env_string("ORDERK_OPENAI_EMBEDDING_MODEL"))
                .or_else(|| env_string("ORDERK_EMBEDDING_MODEL"))
                .unwrap_or_else(|| "text-embedding-3-small".to_string()),
            env_usize("ORDERK_SWORD_EMBEDDING_OPENAI_DIM")
                .or_else(|| env_usize("ORDERK_SWORD_EMBEDDING_DIM"))
                .or_else(|| env_usize("ORDERK_EMBEDDING_DIM"))
                .unwrap_or(1536),
            first_configured_env(&[
                "ORDERK_SWORD_EMBEDDING_OPENAI_API_KEY",
                "ORDERK_OPENAI_API_KEY",
                "ORDERK_EMBEDDING_API_KEY",
            ]),
            env_string("ORDERK_SWORD_EMBEDDING_OPENAI_BASE_URL")
                .or_else(|| env_string("ORDERK_OPENAI_BASE_URL")),
        ),
        "mock" => (
            env_string("ORDERK_SWORD_EMBEDDING_MODEL")
                .or_else(|| env_string("ORDERK_EMBEDDING_MODEL"))
                .unwrap_or_else(|| "mock-8".to_string()),
            env_usize("ORDERK_SWORD_EMBEDDING_DIM")
                .or_else(|| env_usize("ORDERK_EMBEDDING_DIM"))
                .unwrap_or(8),
            None,
            None,
        ),
        other => return Err(anyhow!("unknown embedding provider: {other}")),
    };
    Ok(build_slot(
        SwordModelKind::Embedding,
        provider,
        model,
        Some(dim),
        api_key_env,
        base_url,
    ))
}

fn resolve_reranker_slot() -> Result<SwordModelSlot> {
    let provider = normalize_provider(
        env_string("ORDERK_SWORD_RERANKER_PROVIDER")
            .unwrap_or_else(|| DEFAULT_RERANKER_PROVIDER.to_string()),
    );
    let (model, api_key_env, base_url) = match provider.as_str() {
        "siliconflow" => (
            env_string("ORDERK_SWORD_RERANKER_SILICONFLOW_MODEL")
                .or_else(|| env_string("ORDERK_SWORD_RERANKER_MODEL"))
                .unwrap_or_else(|| DEFAULT_RERANKER_MODEL.to_string()),
            first_configured_env(&[
                "ORDERK_SWORD_RERANKER_SILICONFLOW_API_KEY",
                "ORDERK_SWORD_RERANKER_API_KEY",
                "ORDERK_SILICONFLOW_API_KEY",
            ]),
            env_string("ORDERK_SWORD_RERANKER_SILICONFLOW_BASE_URL")
                .or_else(|| env_string("ORDERK_SWORD_RERANKER_BASE_URL")),
        ),
        "none" | "disabled" => ("none".to_string(), None, None),
        other => return Err(anyhow!("unknown reranker provider: {other}")),
    };
    Ok(build_slot(
        SwordModelKind::Reranker,
        provider,
        model,
        None,
        api_key_env,
        base_url,
    ))
}

fn resolve_llm_slot() -> Result<SwordModelSlot> {
    let provider = normalize_provider(
        env_string("ORDERK_SWORD_LLM_PROVIDER").unwrap_or_else(|| DEFAULT_LLM_PROVIDER.to_string()),
    );
    let (provider, model, api_key_env, base_url) = match provider.as_str() {
        "anthropic" | "minimax" => (
            "anthropic".to_string(),
            env_string("ORDERK_SWORD_LLM_ANTHROPIC_MODEL")
                .or_else(|| env_string("ORDERK_SWORD_LLM_MINIMAX_MODEL"))
                .or_else(|| env_string("ORDERK_SWORD_LLM_MODEL"))
                .unwrap_or_else(|| DEFAULT_LLM_MODEL.to_string()),
            configured_env_pointer("ORDERK_SWORD_LLM_API_KEY_ENV").or_else(|| {
                first_configured_env(&[
                    "ORDERK_SWORD_LLM_ANTHROPIC_API_KEY",
                    "ORDERK_SWORD_LLM_MINIMAX_API_KEY",
                    "ORDERK_SWORD_LLM_API_KEY",
                ])
            }),
            env_string("ORDERK_SWORD_LLM_ANTHROPIC_BASE_URL")
                .or_else(|| env_string("ORDERK_SWORD_LLM_MINIMAX_BASE_URL"))
                .or_else(|| env_string("ORDERK_SWORD_LLM_BASE_URL")),
        ),
        "none" | "disabled" => ("disabled".to_string(), "none".to_string(), None, None),
        other => return Err(anyhow!("unknown llm provider: {other}")),
    };
    Ok(build_slot(
        SwordModelKind::Llm,
        provider,
        model,
        None,
        api_key_env,
        base_url,
    ))
}

fn build_slot(
    kind: SwordModelKind,
    provider: String,
    model: String,
    dim: Option<usize>,
    api_key_env: Option<String>,
    base_url: Option<String>,
) -> SwordModelSlot {
    let api_key_configured = api_key_env.is_some();
    let profile_fingerprint =
        profile_fingerprint(kind, &provider, &model, dim, base_url.as_deref());
    SwordModelSlot {
        kind,
        provider,
        model,
        dim,
        api_key_env,
        api_key_configured,
        base_url,
        profile_fingerprint,
    }
}

fn profile_fingerprint(
    kind: SwordModelKind,
    provider: &str,
    model: &str,
    dim: Option<usize>,
    base_url: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(provider.as_bytes());
    hasher.update(b"\0");
    hasher.update(model.as_bytes());
    hasher.update(b"\0");
    hasher.update(
        dim.map(|value| value.to_string())
            .unwrap_or_default()
            .as_bytes(),
    );
    hasher.update(b"\0");
    hasher.update(base_url.unwrap_or_default().as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn normalize_provider(value: String) -> String {
    value.trim().to_ascii_lowercase().replace('_', "-")
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_usize(name: &str) -> Option<usize> {
    env_string(name).and_then(|value| value.parse::<usize>().ok())
}

fn first_configured_env(names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| env_string(name).map(|_| (*name).to_string()))
}

fn configured_env_pointer(pointer_name: &str) -> Option<String> {
    let pointed = env_string(pointer_name)?;
    if env_string(&pointed).is_some() {
        Some(pointed)
    } else {
        None
    }
}

#[cfg(test)]
mod slot_tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    const SLOT_ENV_NAMES: &[&str] = &[
        "ORDERK_SWORD_EMBEDDING_PROVIDER",
        "ORDERK_SWORD_EMBEDDING_SILICONFLOW_API_KEY",
        "ORDERK_SWORD_EMBEDDING_SILICONFLOW_MODEL",
        "ORDERK_SWORD_EMBEDDING_SILICONFLOW_DIM",
        "ORDERK_SWORD_EMBEDDING_SILICONFLOW_BASE_URL",
        "ORDERK_SWORD_EMBEDDING_OPENAI_API_KEY",
        "ORDERK_SWORD_EMBEDDING_OPENAI_MODEL",
        "ORDERK_SWORD_EMBEDDING_OPENAI_DIM",
        "ORDERK_SWORD_RERANKER_PROVIDER",
        "ORDERK_SWORD_RERANKER_SILICONFLOW_API_KEY",
        "ORDERK_SWORD_RERANKER_SILICONFLOW_MODEL",
        "ORDERK_SWORD_LLM_PROVIDER",
        "ORDERK_SWORD_LLM_ANTHROPIC_API_KEY",
        "ORDERK_SWORD_LLM_ANTHROPIC_MODEL",
        "ORDERK_SWORD_LLM_ANTHROPIC_BASE_URL",
        "ORDERK_SWORD_LLM_MINIMAX_API_KEY",
        "ORDERK_SWORD_LLM_MINIMAX_MODEL",
        "ORDERK_SWORD_LLM_MINIMAX_BASE_URL",
        "ORDERK_SWORD_EMBEDDING_MODEL",
        "ORDERK_SWORD_EMBEDDING_DIM",
        "ORDERK_SWORD_RERANKER_MODEL",
        "ORDERK_SWORD_LLM_MODEL",
        "ORDERK_SWORD_LLM_BASE_URL",
        "ORDERK_EMBEDDING_PROVIDER",
        "ORDERK_EMBEDDING_MODEL",
        "ORDERK_EMBEDDING_DIM",
        "ORDERK_SILICONFLOW_API_KEY",
        "ORDERK_SWORD_RERANKER_API_KEY",
        "ORDERK_SWORD_LLM_API_KEY",
        "ORDERK_SWORD_LLM_API_KEY_ENV",
    ];

    fn with_saved_env<T>(names: &[&str], f: impl FnOnce() -> T) -> T {
        let saved = names
            .iter()
            .map(|name| (*name, std::env::var(name).ok()))
            .collect::<Vec<_>>();
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

    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn with_clean_slot_env<T>(f: impl FnOnce() -> T) -> T {
        let _guard = env_lock();
        let saved = SLOT_ENV_NAMES
            .iter()
            .map(|name| (*name, std::env::var(name).ok()))
            .collect::<Vec<_>>();
        for name in SLOT_ENV_NAMES {
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
    fn slot_provider_resolves_siliconflow_embedding_with_explicit_env() {
        with_clean_slot_env(|| {
            std::env::set_var("ORDERK_SWORD_EMBEDDING_PROVIDER", "siliconflow");
            std::env::set_var("ORDERK_SWORD_EMBEDDING_SILICONFLOW_API_KEY", "sf-key");
            std::env::set_var(
                "ORDERK_SWORD_EMBEDDING_SILICONFLOW_MODEL",
                "Qwen/Qwen3-Embedding-8B",
            );
            std::env::set_var("ORDERK_SWORD_EMBEDDING_SILICONFLOW_DIM", "2048");
            std::env::set_var(
                "ORDERK_SWORD_EMBEDDING_SILICONFLOW_BASE_URL",
                "https://sf.example/v1/embeddings",
            );

            let profile = resolve_sword_model_profile_from_env().unwrap();
            assert_eq!(profile.embedding.provider, "siliconflow");
            assert_eq!(profile.embedding.model, "Qwen/Qwen3-Embedding-8B");
            assert_eq!(profile.embedding.dim, Some(2048));
            assert_eq!(
                profile.embedding.api_key_env.as_deref(),
                Some("ORDERK_SWORD_EMBEDDING_SILICONFLOW_API_KEY")
            );
            assert_eq!(
                profile.embedding.base_url.as_deref(),
                Some("https://sf.example/v1/embeddings")
            );
            assert!(profile.embedding.profile_fingerprint.starts_with("sha256:"));
            assert!(!profile.embedding.profile_fingerprint.contains("sf-key"));
        });
    }

    #[test]
    fn slot_provider_resolves_openai_embedding_when_provider_openai() {
        with_clean_slot_env(|| {
            std::env::set_var("ORDERK_SWORD_EMBEDDING_PROVIDER", "openai");
            std::env::set_var("ORDERK_SWORD_EMBEDDING_OPENAI_API_KEY", "openai-key");
            std::env::set_var(
                "ORDERK_SWORD_EMBEDDING_OPENAI_MODEL",
                "text-embedding-3-large",
            );
            std::env::set_var("ORDERK_SWORD_EMBEDDING_OPENAI_DIM", "3072");

            let slot = resolve_sword_model_slot_from_env(SwordModelKind::Embedding).unwrap();
            assert_eq!(slot.provider, "openai");
            assert_eq!(slot.model, "text-embedding-3-large");
            assert_eq!(slot.dim, Some(3072));
            assert_eq!(
                slot.api_key_env.as_deref(),
                Some("ORDERK_SWORD_EMBEDDING_OPENAI_API_KEY")
            );
            assert!(slot.api_key_configured);
        });
    }

    #[test]
    fn slot_provider_errors_on_unknown_provider() {
        with_clean_slot_env(|| {
            std::env::set_var("ORDERK_SWORD_EMBEDDING_PROVIDER", "made-up-ai");
            let err = resolve_sword_model_slot_from_env(SwordModelKind::Embedding).unwrap_err();
            assert!(
                err.to_string().contains("unknown embedding provider"),
                "{err:#}"
            );
        });
    }

    #[test]
    fn slot_provider_default_falls_back_to_legacy_default_sword_paths() {
        with_clean_slot_env(|| {
            let profile = resolve_sword_model_profile_from_env().unwrap();
            assert_eq!(profile.embedding.provider, "siliconflow");
            assert_eq!(profile.embedding.model, "Qwen/Qwen3-Embedding-4B");
            assert_eq!(profile.embedding.dim, Some(1024));
            assert_eq!(profile.reranker.provider, "siliconflow");
            assert_eq!(profile.reranker.model, "Qwen/Qwen3-Reranker-4B");
            assert_eq!(profile.llm.provider, "anthropic");
            assert_eq!(profile.llm.model, "MiniMax-M3");
        });
    }

    #[test]
    fn slot_profile_ignores_non_orderk_provider_env_names() {
        with_clean_slot_env(|| {
            with_saved_env(
                &[
                    "HERMES_MINIMAX_API_KEY",
                    "HINDSIGHT_API_LLM_API_KEY",
                    "HINDSIGHT_API_LLM_PROVIDER",
                    "HINDSIGHT_API_LLM_MODEL",
                    "HINDSIGHT_API_LLM_BASE_URL",
                    "HINDSIGHT_API_RERANKER_PROVIDER",
                    "HINDSIGHT_API_RERANKER_SILICONFLOW_API_KEY",
                    "HINDSIGHT_API_RERANKER_SILICONFLOW_MODEL",
                    "HINDSIGHT_API_RERANKER_SILICONFLOW_BASE_URL",
                    "HINDSIGHT_API_EMBEDDING_SILICONFLOW_MODEL",
                ],
                || {
                    std::env::set_var("HERMES_MINIMAX_API_KEY", "hermes-minimax-secret");
                    std::env::set_var("HINDSIGHT_API_LLM_API_KEY", "hindsight-llm-secret");
                    std::env::set_var("HINDSIGHT_API_LLM_PROVIDER", "disabled");
                    std::env::set_var("HINDSIGHT_API_LLM_MODEL", "hindsight-llm-model");
                    std::env::set_var(
                        "HINDSIGHT_API_LLM_BASE_URL",
                        "https://hindsight.example/llm",
                    );
                    std::env::set_var("HINDSIGHT_API_RERANKER_PROVIDER", "disabled");
                    std::env::set_var(
                        "HINDSIGHT_API_RERANKER_SILICONFLOW_API_KEY",
                        "hindsight-reranker-secret",
                    );
                    std::env::set_var(
                        "HINDSIGHT_API_RERANKER_SILICONFLOW_MODEL",
                        "hindsight-reranker-model",
                    );
                    std::env::set_var(
                        "HINDSIGHT_API_RERANKER_SILICONFLOW_BASE_URL",
                        "https://hindsight.example/reranker",
                    );
                    std::env::set_var(
                        "HINDSIGHT_API_EMBEDDING_SILICONFLOW_MODEL",
                        "hindsight-embedding-model",
                    );

                    let profile = resolve_sword_model_profile_from_env().unwrap();
                    assert_eq!(profile.embedding.model, DEFAULT_EMBEDDING_MODEL);
                    assert_eq!(profile.reranker.provider, "siliconflow");
                    assert_eq!(profile.reranker.model, DEFAULT_RERANKER_MODEL);
                    assert_eq!(profile.reranker.api_key_env, None);
                    assert_eq!(profile.reranker.base_url, None);
                    assert_eq!(profile.llm.provider, "anthropic");
                    assert_eq!(profile.llm.model, DEFAULT_LLM_MODEL);
                    assert_eq!(profile.llm.api_key_env, None);
                    assert_eq!(profile.llm.base_url, None);
                },
            );
        });
    }

    #[test]
    fn slot_provider_accepts_llm_api_key_env_pointer_without_secret_leak() {
        with_clean_slot_env(|| {
            with_saved_env(&["HERMES_MINIMAX_API_KEY"], || {
                std::env::set_var("HERMES_MINIMAX_API_KEY", "hermes-minimax-secret");
                std::env::set_var("ORDERK_SWORD_LLM_API_KEY_ENV", "HERMES_MINIMAX_API_KEY");
                let slot = resolve_sword_model_slot_from_env(SwordModelKind::Llm).unwrap();
                assert_eq!(slot.provider, "anthropic");
                assert_eq!(slot.model, DEFAULT_LLM_MODEL);
                assert_eq!(slot.api_key_env.as_deref(), Some("HERMES_MINIMAX_API_KEY"));
                assert!(slot.api_key_configured);
                assert!(!slot.profile_fingerprint.contains("hermes-minimax-secret"));
            });
        });
    }

    #[test]
    fn slot_provider_does_not_treat_llm_api_key_env_pointer_as_direct_secret() {
        with_clean_slot_env(|| {
            with_saved_env(&["HERMES_MINIMAX_API_KEY"], || {
                std::env::remove_var("HERMES_MINIMAX_API_KEY");
                std::env::set_var("ORDERK_SWORD_LLM_API_KEY_ENV", "HERMES_MINIMAX_API_KEY");
                let slot = resolve_sword_model_slot_from_env(SwordModelKind::Llm).unwrap();
                assert_eq!(slot.provider, "anthropic");
                assert_eq!(slot.model, DEFAULT_LLM_MODEL);
                assert_eq!(slot.api_key_env, None);
                assert!(!slot.api_key_configured);
            });
        });
    }

    #[test]
    fn slot_provider_independent_per_kind() {
        with_clean_slot_env(|| {
            std::env::set_var("ORDERK_SWORD_EMBEDDING_PROVIDER", "openai");
            std::env::set_var("ORDERK_SWORD_EMBEDDING_OPENAI_API_KEY", "openai-key");
            std::env::set_var(
                "ORDERK_SWORD_EMBEDDING_OPENAI_MODEL",
                "text-embedding-3-small",
            );
            std::env::set_var("ORDERK_SWORD_EMBEDDING_OPENAI_DIM", "1536");
            std::env::set_var("ORDERK_SWORD_RERANKER_PROVIDER", "siliconflow");
            std::env::set_var("ORDERK_SWORD_RERANKER_SILICONFLOW_API_KEY", "sf-rerank-key");
            std::env::set_var(
                "ORDERK_SWORD_RERANKER_SILICONFLOW_MODEL",
                "Qwen/Qwen3-Reranker-8B",
            );
            std::env::set_var("ORDERK_SWORD_LLM_PROVIDER", "anthropic");
            std::env::set_var("ORDERK_SWORD_LLM_ANTHROPIC_API_KEY", "llm-key");
            std::env::set_var("ORDERK_SWORD_LLM_ANTHROPIC_MODEL", "MiniMax-M3-Pro");

            let profile = resolve_sword_model_profile_from_env().unwrap();
            assert_eq!(profile.embedding.provider, "openai");
            assert_eq!(profile.embedding.model, "text-embedding-3-small");
            assert_eq!(profile.embedding.dim, Some(1536));
            assert_eq!(profile.reranker.provider, "siliconflow");
            assert_eq!(profile.reranker.model, "Qwen/Qwen3-Reranker-8B");
            assert_eq!(profile.llm.provider, "anthropic");
            assert_eq!(profile.llm.model, "MiniMax-M3-Pro");
            assert_ne!(
                profile.embedding.profile_fingerprint,
                profile.reranker.profile_fingerprint
            );
            assert_ne!(
                profile.reranker.profile_fingerprint,
                profile.llm.profile_fingerprint
            );
        });
    }
}
