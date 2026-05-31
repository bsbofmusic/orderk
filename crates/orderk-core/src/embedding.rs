use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::thread;
use std::time::Duration;

pub trait EmbeddingProvider: Send + Sync {
    fn provider_id(&self) -> &str;
    fn model_id(&self) -> &str;
    fn dimension(&self) -> usize;
    fn health(&self) -> Result<()>;
    fn embed_documents(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>>;

    fn embed_query(&self, input: &str) -> Result<Vec<f32>> {
        let mut out = self.embed_documents(&[input.to_string()])?;
        out.pop()
            .ok_or_else(|| anyhow!("embedding provider returned no query vector"))
    }
}

#[derive(Debug, Clone)]
pub struct MockEmbeddingProvider {
    dim: usize,
    model: String,
}

impl MockEmbeddingProvider {
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            model: format!("mock-{}", dim),
        }
    }

    fn embed_one(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0_f32; self.dim];
        for token in tokenize(text) {
            let mut hasher = DefaultHasher::new();
            token.hash(&mut hasher);
            let h = hasher.finish();
            let idx = (h as usize) % self.dim;
            let sign = if (h >> 63) == 0 { 1.0 } else { -1.0 };
            let weight = 1.0 + ((h % 7) as f32) / 10.0;
            v[idx] += sign * weight;
        }
        normalize(&mut v);
        v
    }
}

impl EmbeddingProvider for MockEmbeddingProvider {
    fn provider_id(&self) -> &str {
        "mock"
    }
    fn model_id(&self) -> &str {
        &self.model
    }
    fn dimension(&self) -> usize {
        self.dim
    }
    fn health(&self) -> Result<()> {
        Ok(())
    }

    fn embed_documents(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(inputs.iter().map(|s| self.embed_one(s)).collect())
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleEmbeddingProvider {
    provider_id: String,
    label: String,
    api_key: String,
    key_hint: String,
    model: String,
    dim: usize,
    base_url: String,
    max_batch_inputs: usize,
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleEmbeddingConfig {
    pub provider_id: String,
    pub label: String,
    pub api_key: String,
    pub key_hint: String,
    pub model: Option<String>,
    pub default_model: String,
    pub dim: usize,
    pub base_url: Option<String>,
    pub default_base_url: String,
}

#[derive(Debug, Clone)]
pub struct SiliconFlowM3Provider {
    inner: OpenAiCompatibleEmbeddingProvider,
}

const ONLINE_EMBEDDING_CONNECT_TIMEOUT_SECS: u64 = 10;
const ONLINE_EMBEDDING_REQUEST_TIMEOUT_SECS: u64 = 60;
const ONLINE_EMBEDDING_MAX_ATTEMPTS: usize = 3;
const ONLINE_EMBEDDING_MAX_BATCH_INPUTS: usize = 64;

#[derive(Debug, Deserialize)]
struct SiliconFlowEmbeddingsResponse {
    data: Vec<SiliconFlowEmbeddingRow>,
}

#[derive(Debug, Deserialize)]
struct SiliconFlowEmbeddingRow {
    embedding: Vec<f32>,
}

impl OpenAiCompatibleEmbeddingProvider {
    pub fn new(config: OpenAiCompatibleEmbeddingConfig) -> Self {
        Self {
            provider_id: config.provider_id,
            label: config.label,
            api_key: config.api_key,
            key_hint: config.key_hint,
            model: config
                .model
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or(config.default_model),
            dim: config.dim,
            base_url: config
                .base_url
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or(config.default_base_url),
            max_batch_inputs: ONLINE_EMBEDDING_MAX_BATCH_INPUTS,
        }
    }

    fn embed_batch(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(ONLINE_EMBEDDING_CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(ONLINE_EMBEDDING_REQUEST_TIMEOUT_SECS))
            .build();
        let auth = format!("Bearer {}", self.api_key);
        let body =
            serde_json::json!({ "model": &self.model, "input": inputs, "dimensions": self.dim })
                .to_string();

        let mut last_error = String::new();
        for attempt in 1..=ONLINE_EMBEDDING_MAX_ATTEMPTS {
            match agent
                .post(&self.base_url)
                .set("Content-Type", "application/json")
                .set("Authorization", &auth)
                .send_string(&body)
            {
                Ok(response) => {
                    let response_body = response
                        .into_string()
                        .with_context(|| format!("read {} embedding response", self.label))?;
                    let parsed: SiliconFlowEmbeddingsResponse =
                        serde_json::from_str(&response_body)
                            .with_context(|| format!("parse {} embedding response", self.label))?;
                    let mut rows = Vec::with_capacity(parsed.data.len());
                    for row in parsed.data {
                        if row.embedding.len() != self.dim {
                            return Err(anyhow!(
                                "embedding dimension mismatch: provider returned {}, configured {}",
                                row.embedding.len(),
                                self.dim
                            ));
                        }
                        let mut embedding = row.embedding;
                        normalize(&mut embedding);
                        rows.push(embedding);
                    }
                    if rows.len() != inputs.len() {
                        return Err(anyhow!(
                            "embedding response count mismatch: got {}, expected {}",
                            rows.len(),
                            inputs.len()
                        ));
                    }
                    return Ok(rows);
                }
                Err(ureq::Error::Status(code, response)) => {
                    let body = response.into_string().unwrap_or_default();
                    let message = format!("HTTP {}: {}", code, summarize_error_body(&body));
                    if should_retry_http_status(code) && attempt < ONLINE_EMBEDDING_MAX_ATTEMPTS {
                        last_error = message;
                        thread::sleep(Duration::from_millis(retry_backoff_ms(attempt)));
                        continue;
                    }
                    return Err(anyhow!(
                        "{} embedding request failed: {}",
                        self.label,
                        message
                    ));
                }
                Err(ureq::Error::Transport(err)) => {
                    let message = err.to_string();
                    if attempt < ONLINE_EMBEDDING_MAX_ATTEMPTS {
                        last_error = message;
                        thread::sleep(Duration::from_millis(retry_backoff_ms(attempt)));
                        continue;
                    }
                    return Err(anyhow!(
                        "{} embedding request failed after {} attempts: {}",
                        self.label,
                        ONLINE_EMBEDDING_MAX_ATTEMPTS,
                        message
                    ));
                }
            }
        }

        Err(anyhow!(
            "{} embedding request failed after {} attempts: {}",
            self.label,
            ONLINE_EMBEDDING_MAX_ATTEMPTS,
            if last_error.is_empty() {
                "unknown error"
            } else {
                &last_error
            }
        ))
    }
}

impl EmbeddingProvider for OpenAiCompatibleEmbeddingProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }
    fn model_id(&self) -> &str {
        &self.model
    }
    fn dimension(&self) -> usize {
        self.dim
    }
    fn health(&self) -> Result<()> {
        if self.api_key.trim().is_empty() {
            return Err(anyhow!(
                "{} embedding API key is missing; set {}",
                self.label,
                self.key_hint
            ));
        }
        if self.base_url.trim().is_empty() {
            return Err(anyhow!("{} embedding base URL is missing", self.label));
        }
        Ok(())
    }

    fn embed_documents(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        self.health()?;
        if inputs.is_empty() {
            return Ok(Vec::new());
        }

        let mut rows = Vec::with_capacity(inputs.len());
        for batch in inputs.chunks(self.max_batch_inputs.max(1)) {
            let mut batch_rows = self.embed_batch(batch)?;
            rows.append(&mut batch_rows);
        }
        if rows.len() != inputs.len() {
            return Err(anyhow!(
                "embedding response count mismatch: got {}, expected {}",
                rows.len(),
                inputs.len()
            ));
        }
        Ok(rows)
    }
}

impl SiliconFlowM3Provider {
    pub fn new(
        api_key: String,
        model: Option<String>,
        dim: usize,
        base_url: Option<String>,
    ) -> Self {
        Self {
            inner: OpenAiCompatibleEmbeddingProvider::new(OpenAiCompatibleEmbeddingConfig {
                provider_id: "siliconflow".to_string(),
                label: "SiliconFlow".to_string(),
                api_key,
                key_hint: "ORDERK_SILICONFLOW_API_KEY".to_string(),
                model,
                default_model: "BAAI/bge-m3".to_string(),
                dim,
                base_url,
                default_base_url: "https://api.siliconflow.cn/v1/embeddings".to_string(),
            }),
        }
    }
}

impl EmbeddingProvider for SiliconFlowM3Provider {
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
        self.inner.embed_documents(inputs)
    }
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

pub fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

pub fn vector_hash(v: &[f32]) -> String {
    let mut hasher = Sha256::new();
    for x in v {
        hasher.update(x.to_le_bytes());
    }
    hex::encode(hasher.finalize())
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn siliconflow_provider_uses_native_http_and_normalizes_embeddings() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 8192];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            assert!(request.contains("POST /v1/embeddings HTTP/1.1"));
            assert!(request
                .to_lowercase()
                .contains("authorization: bearer test-key"));
            assert!(request.contains("application/json"));

            let response = r#"{"data":[{"embedding":[3.0,0.0,4.0]}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.len(),
                response
            )
            .unwrap();
        });

        let provider = SiliconFlowM3Provider::new(
            "test-key".to_string(),
            Some("fixture-embedding-model".to_string()),
            3,
            Some(format!("http://{}/v1/embeddings", addr)),
        );
        let vectors = provider
            .embed_documents(&["hello world".to_string()])
            .unwrap();
        assert_eq!(vectors.len(), 1);
        assert!((vectors[0][0] - 0.6).abs() < 1e-6, "{:?}", vectors[0]);
        assert!((vectors[0][2] - 0.8).abs() < 1e-6, "{:?}", vectors[0]);

        server.join().unwrap();
    }
}
