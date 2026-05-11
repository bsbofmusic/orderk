
use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub trait EmbeddingProvider: Send + Sync {
    fn provider_id(&self) -> &str;
    fn model_id(&self) -> &str;
    fn dimension(&self) -> usize;
    fn health(&self) -> Result<()>;
    fn embed_documents(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>>;

    fn embed_query(&self, input: &str) -> Result<Vec<f32>> {
        let mut out = self.embed_documents(&[input.to_string()])?;
        out.pop().ok_or_else(|| anyhow!("embedding provider returned no query vector"))
    }
}

#[derive(Debug, Clone)]
pub struct MockEmbeddingProvider {
    dim: usize,
    model: String,
}

impl MockEmbeddingProvider {
    pub fn new(dim: usize) -> Self {
        Self { dim, model: format!("mock-{}", dim) }
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
    fn provider_id(&self) -> &str { "mock" }
    fn model_id(&self) -> &str { &self.model }
    fn dimension(&self) -> usize { self.dim }
    fn health(&self) -> Result<()> { Ok(()) }

    fn embed_documents(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(inputs.iter().map(|s| self.embed_one(s)).collect())
    }
}

#[derive(Debug, Clone)]
pub struct SiliconFlowM3Provider {
    api_key: String,
    model: String,
    dim: usize,
    base_url: String,
}

impl SiliconFlowM3Provider {
    pub fn new(api_key: String, model: Option<String>, dim: usize, base_url: Option<String>) -> Self {
        Self {
            api_key,
            model: model.unwrap_or_else(|| "BAAI/bge-m3".to_string()),
            dim,
            base_url: base_url.unwrap_or_else(|| "https://api.siliconflow.cn/v1/embeddings".to_string()),
        }
    }
}

impl EmbeddingProvider for SiliconFlowM3Provider {
    fn provider_id(&self) -> &str { "siliconflow" }
    fn model_id(&self) -> &str { &self.model }
    fn dimension(&self) -> usize { self.dim }
    fn health(&self) -> Result<()> {
        if self.api_key.trim().is_empty() {
            return Err(anyhow!("SiliconFlow API key is missing; set HERMES_SILICONFLOW_API_KEY or SILICONFLOW_API_KEY"));
        }
        Ok(())
    }

    fn embed_documents(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        self.health()?;
        let body = serde_json::json!({ "model": self.model, "input": inputs }).to_string();
        let script = r#"
import json, os, sys, urllib.request
url = os.environ['ORDERK_SILICONFLOW_URL']
key = os.environ['ORDERK_SILICONFLOW_KEY']
body = sys.stdin.read().encode('utf-8')
req = urllib.request.Request(url, data=body, headers={
    'Content-Type': 'application/json',
    'Authorization': 'Bearer ' + key,
})
with urllib.request.urlopen(req, timeout=60) as resp:
    sys.stdout.write(resp.read().decode('utf-8'))
"#;
        let mut child = std::process::Command::new("python3")
            .arg("-c")
            .arg(script)
            .env("ORDERK_SILICONFLOW_URL", &self.base_url)
            .env("ORDERK_SILICONFLOW_KEY", &self.api_key)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("failed to start python3 for SiliconFlow request: {}", e))?;
        {
            use std::io::Write;
            child.stdin.as_mut().ok_or_else(|| anyhow!("python stdin unavailable"))?.write_all(body.as_bytes())?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("SiliconFlow embedding request failed: {}", stderr.trim()));
        }
        let parsed: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let data = parsed.get("data").and_then(|v| v.as_array()).ok_or_else(|| anyhow!("SiliconFlow response missing data array"))?;
        let mut rows = Vec::with_capacity(data.len());
        for item in data {
            let arr = item.get("embedding").and_then(|v| v.as_array()).ok_or_else(|| anyhow!("SiliconFlow response item missing embedding"))?;
            let mut v = Vec::with_capacity(arr.len());
            for x in arr {
                v.push(x.as_f64().ok_or_else(|| anyhow!("embedding element was not numeric"))? as f32);
            }
            if v.len() != self.dim {
                return Err(anyhow!("embedding dimension mismatch: provider returned {}, configured {}", v.len(), self.dim));
            }
            normalize(&mut v);
            rows.push(v);
        }
        if rows.len() != inputs.len() {
            return Err(anyhow!("embedding response count mismatch: got {}, expected {}", rows.len(), inputs.len()));
        }
        Ok(rows)
    }
}

pub fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() { *x /= norm; }
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
