use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::json;
use std::thread;
use std::time::Duration;

pub const DEFAULT_SEARCH_RERANKER_PROVIDER: &str = "siliconflow";
pub const DEFAULT_SEARCH_RERANKER_MODEL: &str = "Qwen/Qwen3-Reranker-4B";
const SEARCH_RERANKER_CONNECT_TIMEOUT_SECS: u64 = 10;
const SEARCH_RERANKER_REQUEST_TIMEOUT_SECS: u64 = 120;
const SEARCH_RERANKER_MAX_ATTEMPTS: usize = 3;

pub trait RerankerProvider: Send + Sync {
    fn provider_id(&self) -> &str;
    fn model_id(&self) -> &str;
    fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<f32>>;
}

#[derive(Debug, Clone)]
pub struct SiliconFlowRerankerProvider {
    api_key: String,
    model: String,
    base_url: String,
}

#[derive(Debug, Deserialize)]
struct RerankResponse {
    results: Vec<RerankResult>,
}

#[derive(Debug, Deserialize)]
struct RerankResult {
    index: usize,
    relevance_score: f32,
}

impl SiliconFlowRerankerProvider {
    pub fn new(api_key: String, model: Option<String>, base_url: Option<String>) -> Self {
        Self {
            api_key,
            model: model
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_SEARCH_RERANKER_MODEL.to_string()),
            base_url: base_url
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "https://api.siliconflow.cn/v1".to_string()),
        }
    }

    fn endpoint(&self) -> String {
        let trimmed = self.base_url.trim_end_matches('/');
        if trimmed.ends_with("/rerank") {
            trimmed.to_string()
        } else {
            format!("{trimmed}/rerank")
        }
    }
}

impl RerankerProvider for SiliconFlowRerankerProvider {
    fn provider_id(&self) -> &str {
        DEFAULT_SEARCH_RERANKER_PROVIDER
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<f32>> {
        if documents.is_empty() {
            return Ok(Vec::new());
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(SEARCH_RERANKER_CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(SEARCH_RERANKER_REQUEST_TIMEOUT_SECS))
            .build();
        let body = json!({
            "model": self.model,
            "query": query,
            "documents": documents,
            "top_k": documents.len(),
            "return_documents": false
        })
        .to_string();
        let auth = format!("Bearer {}", self.api_key);
        let mut last_error = String::new();
        for attempt in 1..=SEARCH_RERANKER_MAX_ATTEMPTS {
            match agent
                .post(&self.endpoint())
                .set("Content-Type", "application/json")
                .set("Authorization", &auth)
                .send_string(&body)
            {
                Ok(response) => {
                    let response_body = response.into_string().context("read reranker response")?;
                    let parsed: RerankResponse =
                        serde_json::from_str(&response_body).context("parse reranker response")?;
                    let mut scores = vec![0.0_f32; documents.len()];
                    for result in parsed.results {
                        if result.index < scores.len() {
                            scores[result.index] = result.relevance_score;
                        }
                    }
                    return Ok(scores);
                }
                Err(ureq::Error::Status(code, response)) => {
                    let body = response.into_string().unwrap_or_default();
                    let message = format!("HTTP {}: {}", code, summarize_error_body(&body));
                    if should_retry_http_status(code) && attempt < SEARCH_RERANKER_MAX_ATTEMPTS {
                        last_error = message;
                        thread::sleep(Duration::from_millis(retry_backoff_ms(attempt)));
                        continue;
                    }
                    return Err(anyhow!(
                        "SiliconFlow Qwen reranker request failed: {message}"
                    ));
                }
                Err(ureq::Error::Transport(err)) => {
                    let message = err.to_string();
                    if attempt < SEARCH_RERANKER_MAX_ATTEMPTS {
                        last_error = message;
                        thread::sleep(Duration::from_millis(retry_backoff_ms(attempt)));
                        continue;
                    }
                    return Err(anyhow!(
                        "SiliconFlow Qwen reranker request failed after 3 attempts: {message}"
                    ));
                }
            }
        }
        Err(anyhow!(
            "SiliconFlow Qwen reranker request failed after 3 attempts: {}",
            if last_error.is_empty() {
                "unknown error"
            } else {
                &last_error
            }
        ))
    }
}

pub fn provider_from_env() -> Result<Box<dyn RerankerProvider>> {
    let provider = env_string("ORDERK_SEARCH_RERANKER_PROVIDER")
        .or_else(|| env_string("ORDERK_RERANKER_PROVIDER"))
        .or_else(|| env_string("ORDERK_SWORD_RERANKER_PROVIDER"))
        .unwrap_or_else(|| DEFAULT_SEARCH_RERANKER_PROVIDER.to_string())
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-");
    match provider.as_str() {
        "siliconflow" => Ok(Box::new(SiliconFlowRerankerProvider::new(
            required_env_any(
                &[
                    "ORDERK_SEARCH_RERANKER_SILICONFLOW_API_KEY",
                    "ORDERK_SEARCH_RERANKER_API_KEY",
                    "ORDERK_RERANKER_SILICONFLOW_API_KEY",
                    "ORDERK_RERANKER_API_KEY",
                    "ORDERK_SWORD_RERANKER_SILICONFLOW_API_KEY",
                    "ORDERK_SWORD_RERANKER_API_KEY",
                    "ORDERK_SILICONFLOW_API_KEY",
                ],
                "SiliconFlow Qwen search reranker",
            )?,
            env_string("ORDERK_SEARCH_RERANKER_SILICONFLOW_MODEL")
                .or_else(|| env_string("ORDERK_SEARCH_RERANKER_MODEL"))
                .or_else(|| env_string("ORDERK_RERANKER_MODEL"))
                .or_else(|| env_string("ORDERK_SWORD_RERANKER_SILICONFLOW_MODEL"))
                .or_else(|| env_string("ORDERK_SWORD_RERANKER_MODEL")),
            env_string("ORDERK_SEARCH_RERANKER_SILICONFLOW_BASE_URL")
                .or_else(|| env_string("ORDERK_SEARCH_RERANKER_BASE_URL"))
                .or_else(|| env_string("ORDERK_RERANKER_BASE_URL"))
                .or_else(|| env_string("ORDERK_SWORD_RERANKER_SILICONFLOW_BASE_URL"))
                .or_else(|| env_string("ORDERK_SWORD_RERANKER_BASE_URL"))
                .or_else(|| env_string("ORDERK_SILICONFLOW_BASE_URL")),
        ))),
        "none" | "disabled" => Err(anyhow!(
            "search reranker provider `{provider}` is disabled; use --reranker none only for explicit test/migration escape hatches"
        )),
        other => Err(anyhow!(
            "unknown search reranker provider: {other} (expected siliconflow; default model is Qwen/Qwen3-Reranker-4B; use --reranker none for the explicit test/migration escape hatch)"
        )),
    }
}

fn required_env_any(names: &[&str], label: &str) -> Result<String> {
    for name in names {
        if let Some(value) = env_string(name) {
            return Ok(value);
        }
    }
    Err(anyhow!(
        "missing {label} API key; set one of {} or use --reranker none for the explicit test/migration escape hatch",
        names.join("|")
    ))
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn should_retry_http_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

fn retry_backoff_ms(attempt: usize) -> u64 {
    250 * 2_u64.saturating_pow((attempt.saturating_sub(1)) as u32)
}

fn summarize_error_body(body: &str) -> String {
    let trimmed = body.trim();
    let preview: String = trimmed.chars().take(300).collect();
    if preview.is_empty() {
        "<empty body>".to_string()
    } else if preview.len() < trimmed.len() {
        format!("{}…", preview)
    } else {
        preview
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn provider_from_env_fails_closed_without_siliconflow_key() {
        let _guard = env_lock();
        let saved = [
            "ORDERK_SEARCH_RERANKER_PROVIDER",
            "ORDERK_SEARCH_RERANKER_SILICONFLOW_API_KEY",
            "ORDERK_SEARCH_RERANKER_API_KEY",
            "ORDERK_RERANKER_SILICONFLOW_API_KEY",
            "ORDERK_RERANKER_API_KEY",
            "ORDERK_SWORD_RERANKER_SILICONFLOW_API_KEY",
            "ORDERK_SWORD_RERANKER_API_KEY",
            "ORDERK_SILICONFLOW_API_KEY",
        ]
        .into_iter()
        .map(|name| (name, std::env::var(name).ok()))
        .collect::<Vec<_>>();
        for (name, _) in &saved {
            std::env::remove_var(name);
        }
        let err = match provider_from_env() {
            Ok(provider) => panic!(
                "provider_from_env should fail closed without a key, got {}:{}",
                provider.provider_id(),
                provider.model_id()
            ),
            Err(err) => err.to_string(),
        };
        assert!(
            err.contains("missing SiliconFlow Qwen search reranker API key"),
            "{err}"
        );
        for (name, value) in saved {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
    }

    #[test]
    fn provider_from_env_rejects_mock_reranker_provider() {
        let _guard = env_lock();
        let saved = [
            "ORDERK_SEARCH_RERANKER_PROVIDER",
            "ORDERK_SEARCH_RERANKER_SILICONFLOW_API_KEY",
            "ORDERK_SILICONFLOW_API_KEY",
        ]
        .into_iter()
        .map(|name| (name, std::env::var(name).ok()))
        .collect::<Vec<_>>();
        for (name, _) in &saved {
            std::env::remove_var(name);
        }
        std::env::set_var("ORDERK_SEARCH_RERANKER_PROVIDER", "mock");
        let err = match provider_from_env() {
            Ok(provider) => panic!(
                "mock reranker provider must not be a runtime escape hatch, got {}:{}",
                provider.provider_id(),
                provider.model_id()
            ),
            Err(err) => err.to_string(),
        };
        assert!(
            err.contains("unknown search reranker provider") || err.contains("mock"),
            "{err}"
        );
        for (name, value) in saved {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
    }

    #[test]
    fn siliconflow_reranker_uses_qwen_model_and_rerank_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_millis(500)))
                .unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0_u8; 8192];
            loop {
                match stream.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        let request = String::from_utf8_lossy(&buf);
                        if request.contains("Qwen/Qwen3-Reranker-4B") {
                            break;
                        }
                    }
                    Err(err)
                        if matches!(
                            err.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        break;
                    }
                    Err(err) => panic!("read reranker request: {err}"),
                }
            }
            let request = String::from_utf8_lossy(&buf);
            assert!(request.contains("POST /v1/rerank HTTP/1.1"), "{request}");
            assert!(
                request
                    .to_lowercase()
                    .contains("authorization: bearer test-key"),
                "{request}"
            );
            assert!(request.contains("Qwen/Qwen3-Reranker-4B"), "{request}");
            let response = r#"{"results":[{"index":1,"relevance_score":0.91},{"index":0,"relevance_score":0.12}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.len(),
                response
            )
            .unwrap();
        });
        let reranker = SiliconFlowRerankerProvider::new(
            "test-key".to_string(),
            None,
            Some(format!("http://{}/v1", addr)),
        );
        let scores = reranker
            .rerank(
                "qwen reranker",
                &["alpha".to_string(), "qwen reranker beta".to_string()],
            )
            .unwrap();
        assert_eq!(scores, vec![0.12, 0.91]);
        server.join().unwrap();
    }
}
