use crate::api::provider_from_name;
use crate::embedding::EmbeddingProvider;
use crate::markdown::parse_markdown;
use crate::models::ScannedFile;
use crate::scanner::scan_vault;
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_MAX_FILES: usize = 200;
const DEFAULT_MAX_PROPOSALS: usize = 100;
const DEFAULT_LLM_PROVIDER: &str = "anthropic";
const DEFAULT_LLM_MODEL: &str = "MiniMax-M3";
const DEFAULT_RERANKER_PROVIDER: &str = "siliconflow";
const DEFAULT_RERANKER_MODEL: &str = "Qwen/Qwen3-Reranker-4B";
const DEFAULT_EMBEDDING_PROVIDER: &str = "siliconflow";
const DEFAULT_EMBEDDING_MODEL: &str = "Qwen/Qwen3-Embedding-4B";
const DEFAULT_EMBEDDING_DIM: usize = 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwordSpiritBudgetProfile {
    DigestLow,
    DigestStandard,
    DigestDeep,
    Eval,
}

impl SwordSpiritBudgetProfile {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "low" | "digest_low" => Ok(Self::DigestLow),
            "standard" | "mid" | "digest_standard" => Ok(Self::DigestStandard),
            "deep" | "high" | "digest_deep" => Ok(Self::DigestDeep),
            "eval" | "bench" | "benchmark" => Ok(Self::Eval),
            other => Err(anyhow!(
                "unknown sword budget profile: {other}; expected digest_low, digest_standard, digest_deep, or eval"
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DigestLow => "digest_low",
            Self::DigestStandard => "digest_standard",
            Self::DigestDeep => "digest_deep",
            Self::Eval => "eval",
        }
    }

    pub fn budget(&self) -> SwordSpiritBudget {
        match self {
            Self::DigestLow => SwordSpiritBudget {
                profile: "digest_low",
                candidate_multiplier: 2,
                candidate_min: 12,
                candidate_max: 24,
                lexical_per_source_cap: 3,
                embedding_per_source_cap: 4,
                reranker_per_source_cap: 12,
                llm_candidate_cap: 12,
                llm_batch_size: 8,
                fallback_threshold: 0.72,
                fallback_policy: "proposal_only_review",
            },
            Self::DigestStandard => SwordSpiritBudget {
                profile: "digest_standard",
                candidate_multiplier: 4,
                candidate_min: 24,
                candidate_max: 160,
                lexical_per_source_cap: 6,
                embedding_per_source_cap: 8,
                reranker_per_source_cap: 24,
                llm_candidate_cap: 40,
                llm_batch_size: 12,
                fallback_threshold: 0.62,
                fallback_policy: "proposal_only_review",
            },
            Self::DigestDeep => SwordSpiritBudget {
                profile: "digest_deep",
                candidate_multiplier: 6,
                candidate_min: 48,
                candidate_max: 320,
                lexical_per_source_cap: 10,
                embedding_per_source_cap: 12,
                reranker_per_source_cap: 40,
                llm_candidate_cap: 80,
                llm_batch_size: 12,
                fallback_threshold: 0.58,
                fallback_policy: "proposal_only_review",
            },
            Self::Eval => SwordSpiritBudget {
                profile: "eval",
                candidate_multiplier: 8,
                candidate_min: 64,
                candidate_max: 512,
                lexical_per_source_cap: 12,
                embedding_per_source_cap: 16,
                reranker_per_source_cap: 64,
                llm_candidate_cap: 120,
                llm_batch_size: 12,
                fallback_threshold: 0.55,
                fallback_policy: "proposal_only_review",
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwordSpiritBudget {
    pub profile: &'static str,
    pub candidate_multiplier: usize,
    pub candidate_min: usize,
    pub candidate_max: usize,
    pub lexical_per_source_cap: usize,
    pub embedding_per_source_cap: usize,
    pub reranker_per_source_cap: usize,
    pub llm_candidate_cap: usize,
    pub llm_batch_size: usize,
    pub fallback_threshold: f32,
    pub fallback_policy: &'static str,
}

impl SwordSpiritBudget {
    pub fn candidate_limit(&self, max_proposals: usize) -> usize {
        max_proposals
            .max(1)
            .saturating_mul(self.candidate_multiplier.max(1))
            .clamp(self.candidate_min.max(1), self.candidate_max.max(1))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwordSpiritTraceLevel {
    Off,
    Compact,
    Full,
}

impl SwordSpiritTraceLevel {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "none" => Ok(Self::Off),
            "compact" | "summary" => Ok(Self::Compact),
            "full" | "debug" => Ok(Self::Full),
            other => Err(anyhow!(
                "unknown sword trace level: {other}; expected off, compact, or full"
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Compact => "compact",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwordSpiritThinkingMode {
    Heuristic,
    Active,
}

impl SwordSpiritThinkingMode {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "heuristic" | "off" | "dry" | "dry-run" => Ok(Self::Heuristic),
            "active" | "live" | "sword" => Ok(Self::Active),
            other => Err(anyhow!(
                "unknown sword thinking mode: {other}; expected heuristic or active"
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::Active => "active",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwordSpiritOptions {
    pub max_files: usize,
    pub max_proposals: usize,
    pub llm_provider: String,
    pub llm_model: String,
    pub thinking_mode: SwordSpiritThinkingMode,
    pub reranker_provider: String,
    pub reranker_model: String,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub embedding_dim: usize,
    pub budget_profile: SwordSpiritBudgetProfile,
    pub trace_level: SwordSpiritTraceLevel,
}

impl Default for SwordSpiritOptions {
    fn default() -> Self {
        Self {
            max_files: DEFAULT_MAX_FILES,
            max_proposals: DEFAULT_MAX_PROPOSALS,
            llm_provider: DEFAULT_LLM_PROVIDER.to_string(),
            llm_model: DEFAULT_LLM_MODEL.to_string(),
            thinking_mode: SwordSpiritThinkingMode::Heuristic,
            reranker_provider: DEFAULT_RERANKER_PROVIDER.to_string(),
            reranker_model: DEFAULT_RERANKER_MODEL.to_string(),
            embedding_provider: DEFAULT_EMBEDDING_PROVIDER.to_string(),
            embedding_model: DEFAULT_EMBEDDING_MODEL.to_string(),
            embedding_dim: DEFAULT_EMBEDDING_DIM,
            budget_profile: SwordSpiritBudgetProfile::DigestStandard,
            trace_level: SwordSpiritTraceLevel::Compact,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwordSpiritRunResponse {
    pub ok: bool,
    pub schema_version: String,
    pub mode: String,
    pub vault: String,
    pub run_id: String,
    pub sidecar_root: String,
    pub run_dir: String,
    pub files_scanned: usize,
    pub files_considered: usize,
    pub proposal_count: usize,
    pub rejected_count: usize,
    pub proposals_path: String,
    pub rejected_path: String,
    pub audit_path: String,
    pub manifest_path: String,
    pub report_path: String,
    pub neighbors_path: String,
    pub boundary: SwordSpiritBoundary,
    pub llm: SwordSpiritLlmMetadata,
    pub thinking: SwordSpiritThinkingMetadata,
    pub proposals: Vec<SwordSpiritProposal>,
    pub rejected: Vec<SwordSpiritRejectedDecision>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwordSpiritStatusResponse {
    pub ok: bool,
    pub schema_version: String,
    pub vault: String,
    pub sidecar_root: String,
    pub runs: usize,
    pub latest_run_id: Option<String>,
    pub latest_run_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwordSpiritBoundary {
    pub markdown_base_owner: String,
    pub orderk_role: String,
    pub writes: Vec<String>,
    pub forbidden: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwordSpiritLlmMetadata {
    pub provider: String,
    pub model: String,
    pub invocation: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SwordSpiritThinkingMetadata {
    pub mode: String,
    pub budget_profile: String,
    pub trace_level: String,
    pub candidate_limit: usize,
    pub lexical_per_source_cap: usize,
    pub embedding_per_source_cap: usize,
    pub reranker_per_source_cap: usize,
    pub llm_candidate_cap: usize,
    pub llm_batch_size: usize,
    pub fallback_threshold: f32,
    pub fallback_policy: String,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub embedding_dim: usize,
    pub embedding_invocation: String,
    pub embedded_count: usize,
    pub reranker_provider: String,
    pub reranker_model: String,
    pub reranker_invocation: String,
    pub llm_invocation: String,
    pub candidate_count: usize,
    pub reranked_count: usize,
    pub accepted_count: usize,
    pub llm_calls: usize,
    pub rejected_count: usize,
    pub fallback_invocation: String,
    pub wall_time_ms: u128,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SwordSpiritProposal {
    pub schema_version: String,
    pub id: String,
    pub proposal_type: String,
    pub relation: Option<String>,
    pub source_path: String,
    pub target_path: Option<String>,
    pub confidence: f32,
    pub risk: String,
    pub auto_apply: bool,
    pub human_review_required: bool,
    pub evidence: Vec<SwordSpiritEvidence>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SwordSpiritEvidence {
    pub path: String,
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SwordSpiritNeighborCandidate {
    pub schema_version: String,
    pub id: String,
    pub source_path: String,
    pub target_path: String,
    pub source_title: Option<String>,
    pub target_title: Option<String>,
    pub evidence_kind: String,
    pub lexical_score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reranker_score: Option<f32>,
    #[serde(default)]
    pub source_ranks: Vec<SwordSpiritSourceRank>,
    #[serde(default)]
    pub rrf_score: f32,
    #[serde(default)]
    pub final_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SwordSpiritSourceRank {
    pub source: String,
    pub rank: usize,
    pub score: f32,
    pub rrf_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SwordSpiritRejectedDecision {
    pub schema_version: String,
    pub candidate_id: String,
    pub source_path: Option<String>,
    pub target_path: Option<String>,
    pub reason: String,
    pub detail: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ActiveProposalDecision {
    candidate_id: String,
    #[serde(default)]
    keep: bool,
    #[serde(default)]
    relation: Option<String>,
    #[serde(default)]
    confidence: Option<f32>,
    #[serde(default)]
    rationale: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SwordSpiritManifest<'a> {
    schema_version: &'a str,
    vault: String,
    files: &'a [ScannedFile],
}

#[derive(Debug, Clone, Serialize)]
struct SwordSpiritAuditEvent<'a> {
    schema_version: &'a str,
    event: &'a str,
    run_id: &'a str,
    proposal_count: usize,
    created_at: String,
    note: &'a str,
}

#[derive(Debug, Clone)]
struct SwordSpiritDocument {
    path: String,
    hash: String,
    title: Option<String>,
    wikilinks: Vec<String>,
    tags: Vec<String>,
    body_excerpt: String,
}

#[derive(Debug, Clone)]
struct SiliconFlowRerankerClient {
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

#[derive(Debug, Clone)]
struct AnthropicMiniMaxClient {
    api_key: String,
    model: String,
    base_url: String,
    calls: usize,
}

pub fn run_sword_spirit(
    vault: &Path,
    options: &SwordSpiritOptions,
) -> Result<SwordSpiritRunResponse> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let sidecar_root = vault.join(".orderk").join("sword_spirit");
    let (run_id, run_dir) = create_unique_run_dir(&sidecar_root)?;

    let files = scan_vault(&vault)?;
    let mut warnings = Vec::new();
    if files.len() > options.max_files {
        warnings.push(format!(
            "files_scanned={} exceeds max_files={}; considered the first sorted subset only",
            files.len(),
            options.max_files
        ));
    }
    let considered: Vec<ScannedFile> = files.iter().take(options.max_files).cloned().collect();
    let documents = read_documents(&considered)?;
    let (mut proposals, mut rejected, neighbors, thinking) =
        generate_sword_output(&documents, options, &mut warnings)?;
    proposals.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    proposals.truncate(options.max_proposals);
    rejected.sort_by(|a, b| {
        a.candidate_id
            .cmp(&b.candidate_id)
            .then_with(|| a.reason.cmp(&b.reason))
    });

    let manifest_path = run_dir.join("input-manifest.json");
    let proposals_path = run_dir.join("proposals.jsonl");
    let rejected_path = run_dir.join("rejected.jsonl");
    let audit_path = run_dir.join("audit.jsonl");
    let report_path = run_dir.join("report.md");
    let neighbors_path = run_dir.join("neighbors.jsonl");

    let manifest = SwordSpiritManifest {
        schema_version: "orderk.sword_spirit.input_manifest.v1",
        vault: vault.to_string_lossy().to_string(),
        files: &considered,
    };
    write_json_pretty(&manifest_path, &manifest)?;
    write_jsonl(&proposals_path, &proposals)?;
    write_jsonl(&rejected_path, &rejected)?;
    write_jsonl(&neighbors_path, &neighbors)?;
    let audit_note = match options.thinking_mode {
        SwordSpiritThinkingMode::Heuristic => {
            "Heuristic proposal run created; no Markdown source files were modified."
        }
        SwordSpiritThinkingMode::Active => {
            "Active Sword Spirit run created with live embedding/reranker/LLM calls; no Markdown source files were modified."
        }
    };
    write_jsonl(
        &audit_path,
        &[SwordSpiritAuditEvent {
            schema_version: "orderk.sword_spirit.audit_event.v1",
            event: "proposal_run_created",
            run_id: &run_id,
            proposal_count: proposals.len(),
            created_at: Utc::now().to_rfc3339(),
            note: audit_note,
        }],
    )?;
    write_report(
        &report_path,
        &run_id,
        &vault,
        &proposals,
        &neighbors,
        options,
        &thinking,
    )?;

    Ok(SwordSpiritRunResponse {
        ok: true,
        schema_version: "orderk.sword_spirit.run.v1".to_string(),
        mode: options.thinking_mode.as_str().to_string(),
        vault: vault.to_string_lossy().to_string(),
        run_id,
        sidecar_root: sidecar_root.to_string_lossy().to_string(),
        run_dir: run_dir.to_string_lossy().to_string(),
        files_scanned: files.len(),
        files_considered: considered.len(),
        proposal_count: proposals.len(),
        rejected_count: rejected.len(),
        proposals_path: proposals_path.to_string_lossy().to_string(),
        rejected_path: rejected_path.to_string_lossy().to_string(),
        audit_path: audit_path.to_string_lossy().to_string(),
        manifest_path: manifest_path.to_string_lossy().to_string(),
        report_path: report_path.to_string_lossy().to_string(),
        neighbors_path: neighbors_path.to_string_lossy().to_string(),
        boundary: sword_spirit_boundary(),
        llm: SwordSpiritLlmMetadata {
            provider: options.llm_provider.clone(),
            model: options.llm_model.clone(),
            invocation: thinking.llm_invocation.clone(),
            note: match options.thinking_mode {
                SwordSpiritThinkingMode::Heuristic => "Heuristic mode records the Hindsight-aligned model choice but does not read API keys.".to_string(),
                SwordSpiritThinkingMode::Active => "Active mode uses the Hindsight-aligned Anthropic-compatible MiniMax M3 path; API keys are read from env and never written to sidecars.".to_string(),
            },
        },
        thinking,
        proposals,
        rejected,
        warnings,
    })
}

pub fn sword_spirit_status(vault: &Path) -> Result<SwordSpiritStatusResponse> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let sidecar_root = vault.join(".orderk").join("sword_spirit");
    let runs_dir = sidecar_root.join("runs");
    let mut runs = Vec::new();
    if runs_dir.exists() {
        for entry in fs::read_dir(&runs_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                runs.push(entry.path());
            }
        }
    }
    runs.sort();
    let latest = runs.last().cloned();
    Ok(SwordSpiritStatusResponse {
        ok: true,
        schema_version: "orderk.sword_spirit.status.v1".to_string(),
        vault: vault.to_string_lossy().to_string(),
        sidecar_root: sidecar_root.to_string_lossy().to_string(),
        runs: runs.len(),
        latest_run_id: latest
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|name| name.to_string_lossy().to_string()),
        latest_run_dir: latest.map(|p| p.to_string_lossy().to_string()),
    })
}

pub fn default_sword_llm_provider() -> String {
    env_string("ORDERK_SWORD_LLM_PROVIDER")
        .or_else(|| env_string("HINDSIGHT_API_LLM_PROVIDER"))
        .unwrap_or_else(|| DEFAULT_LLM_PROVIDER.to_string())
}

pub fn default_sword_llm_model() -> String {
    env_string("ORDERK_SWORD_LLM_MODEL")
        .or_else(|| env_string("HINDSIGHT_API_LLM_MODEL"))
        .unwrap_or_else(|| DEFAULT_LLM_MODEL.to_string())
}

pub fn default_sword_reranker_provider() -> String {
    env_string("ORDERK_SWORD_RERANKER_PROVIDER")
        .or_else(|| env_string("HINDSIGHT_API_RERANKER_PROVIDER"))
        .unwrap_or_else(|| DEFAULT_RERANKER_PROVIDER.to_string())
}

pub fn default_sword_reranker_model() -> String {
    env_string("ORDERK_SWORD_RERANKER_MODEL")
        .or_else(|| env_string("HINDSIGHT_API_RERANKER_SILICONFLOW_MODEL"))
        .unwrap_or_else(|| DEFAULT_RERANKER_MODEL.to_string())
}

pub fn default_sword_embedding_provider() -> String {
    env_string("ORDERK_SWORD_EMBEDDING_PROVIDER")
        .or_else(|| env_string("ORDERK_EMBEDDING_PROVIDER"))
        .unwrap_or_else(|| DEFAULT_EMBEDDING_PROVIDER.to_string())
}

pub fn default_sword_embedding_model() -> String {
    env_string("ORDERK_SWORD_EMBEDDING_MODEL")
        .or_else(|| env_string("HINDSIGHT_API_EMBEDDING_SILICONFLOW_MODEL"))
        .or_else(|| env_string("ORDERK_EMBEDDING_MODEL"))
        .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_string())
}

pub fn default_sword_embedding_dim() -> usize {
    env_string("ORDERK_SWORD_EMBEDDING_DIM")
        .or_else(|| env_string("ORDERK_EMBEDDING_DIM"))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_EMBEDDING_DIM)
}

type SwordSpiritGeneratedOutput = (
    Vec<SwordSpiritProposal>,
    Vec<SwordSpiritRejectedDecision>,
    Vec<SwordSpiritNeighborCandidate>,
    SwordSpiritThinkingMetadata,
);

fn generate_sword_output(
    documents: &[SwordSpiritDocument],
    options: &SwordSpiritOptions,
    warnings: &mut Vec<String>,
) -> Result<SwordSpiritGeneratedOutput> {
    let budget = options.budget_profile.budget();
    match options.thinking_mode {
        SwordSpiritThinkingMode::Heuristic => {
            let proposals = generate_heuristic_proposals(documents, options.max_proposals);
            let thinking = base_thinking_metadata(
                options,
                &budget,
                budget.candidate_limit(options.max_proposals),
                "heuristic",
            );
            let thinking = SwordSpiritThinkingMetadata {
                embedding_invocation: "not_called_heuristic".to_string(),
                reranker_invocation: "not_called_heuristic".to_string(),
                llm_invocation: "not_called_heuristic".to_string(),
                candidate_count: proposals.len(),
                accepted_count: proposals.len(),
                note: format!(
                    "Heuristic compatibility mode; use --thinking active to run live Sword Spirit thinking. budget_profile={} trace_level={}",
                    budget.profile,
                    options.trace_level.as_str()
                ),
                ..thinking
            };
            Ok((proposals, Vec::new(), Vec::new(), thinking))
        }
        SwordSpiritThinkingMode::Active => generate_active_proposals(documents, options, warnings),
    }
}

fn generate_active_proposals(
    documents: &[SwordSpiritDocument],
    options: &SwordSpiritOptions,
    warnings: &mut Vec<String>,
) -> Result<SwordSpiritGeneratedOutput> {
    if !options
        .reranker_provider
        .trim()
        .eq_ignore_ascii_case(DEFAULT_RERANKER_PROVIDER)
    {
        return Err(anyhow!(
            "active Sword Spirit currently supports reranker provider `{}` only; got `{}`",
            DEFAULT_RERANKER_PROVIDER,
            options.reranker_provider
        ));
    }
    if !options
        .llm_provider
        .trim()
        .eq_ignore_ascii_case(DEFAULT_LLM_PROVIDER)
    {
        return Err(anyhow!(
            "active Sword Spirit currently supports Anthropic-compatible LLM provider `{}` only; got `{}`",
            DEFAULT_LLM_PROVIDER,
            options.llm_provider
        ));
    }

    let started = Instant::now();
    let budget = options.budget_profile.budget();
    let candidate_limit = budget.candidate_limit(options.max_proposals);
    let embedding_provider = provider_from_name(
        &options.embedding_provider,
        options.embedding_dim,
        Some(options.embedding_model.clone()),
    )?;
    let mut neighbors = generate_neighbor_candidates_with_limit(
        documents,
        &budget,
        candidate_limit,
        Some(embedding_provider.as_ref()),
    )?;
    let candidate_count = neighbors.len();
    if neighbors.is_empty() {
        let thinking = SwordSpiritThinkingMetadata {
            embedding_invocation: "called_no_candidates".to_string(),
            embedded_count: documents.len(),
            reranker_invocation: "not_called_no_candidates".to_string(),
            llm_invocation: "not_called_no_candidates".to_string(),
            note: format!(
                "No candidate document pairs were available for active thinking. budget_profile={} trace_level={}",
                budget.profile,
                options.trace_level.as_str()
            ),
            ..base_thinking_metadata(options, &budget, candidate_limit, "active")
        };
        return Ok((Vec::new(), Vec::new(), neighbors, thinking));
    }

    let reranker = SiliconFlowRerankerClient::from_env(&options.reranker_model)?;
    rerank_neighbors(&reranker, documents, &mut neighbors, &budget, warnings)?;
    sort_neighbors(&mut neighbors);
    neighbors.truncate(candidate_limit);

    let llm_candidates: Vec<SwordSpiritNeighborCandidate> = neighbors
        .iter()
        .take(
            options
                .max_proposals
                .clamp(1, budget.llm_candidate_cap.max(1)),
        )
        .cloned()
        .collect();
    let mut llm = AnthropicMiniMaxClient::from_env(&options.llm_model)?;
    let (decisions, llm_invocation) = match llm.decide_candidates(
        &llm_candidates,
        documents,
        options.max_proposals,
        &budget,
    ) {
        Ok(decisions) => (decisions, "called".to_string()),
        Err(err) => {
            warnings.push(format!(
                    "active LLM was called but returned no parseable decisions; preserving embedding+reranker neighbors as review-only proposals: {err}"
                ));
            (Vec::new(), "called_unparseable_fallback".to_string())
        }
    };
    let (mut proposals, rejected) = proposals_from_decisions(
        &decisions,
        &llm_candidates,
        documents,
        options.max_proposals,
    );
    let used_reranker_fallback =
        proposals.is_empty() && should_use_reranker_fallback(&llm_invocation);
    if used_reranker_fallback {
        warnings.push(format!(
            "active LLM did not produce parseable candidate decisions; falling back to high-confidence reranker neighbors as review-only proposals using threshold {:.2}",
            budget.fallback_threshold
        ));
        proposals = llm_candidates
            .iter()
            .filter(|candidate| candidate_rank_score(candidate) >= budget.fallback_threshold)
            .take(options.max_proposals)
            .map(reranker_fallback_proposal)
            .collect();
    }

    let thinking = SwordSpiritThinkingMetadata {
        embedding_invocation: "called".to_string(),
        embedded_count: documents.len(),
        reranker_invocation: "called".to_string(),
        llm_invocation: llm_invocation.clone(),
        candidate_count,
        reranked_count: neighbors
            .iter()
            .filter(|candidate| candidate.reranker_score.is_some())
            .count(),
        accepted_count: proposals.len(),
        llm_calls: llm.calls,
        rejected_count: rejected.len(),
        fallback_invocation: if used_reranker_fallback {
            "proposal_only_review".to_string()
        } else {
            "not_used".to_string()
        },
        wall_time_ms: started.elapsed().as_millis(),
        note: format!(
            "Active mode embeds documents with the HS-aligned Qwen3 embedding profile, generates embedding/lexical candidates, reranks them with the HS-aligned SiliconFlow Qwen3 reranker, then asks MiniMax M3 to keep typed semantic edges; fallback is explicit. budget_profile={} trace_level={}",
            budget.profile,
            options.trace_level.as_str()
        ),
        ..base_thinking_metadata(options, &budget, candidate_limit, "active")
    };
    Ok((proposals, rejected, neighbors, thinking))
}

fn read_documents(files: &[ScannedFile]) -> Result<Vec<SwordSpiritDocument>> {
    let mut docs = Vec::new();
    for file in files {
        let bytes = fs::read(&file.abs_path)
            .with_context(|| format!("read markdown source: {}", file.abs_path.display()))?;
        let source = String::from_utf8_lossy(&bytes);
        let parsed = parse_markdown(&file.path, &source)?;
        docs.push(SwordSpiritDocument {
            path: parsed.path,
            hash: file.hash.clone(),
            title: parsed.title,
            wikilinks: parsed.wikilinks,
            tags: parsed.tags,
            body_excerpt: compact_excerpt(&parsed.body, 1400),
        });
    }
    Ok(docs)
}

fn generate_heuristic_proposals(
    documents: &[SwordSpiritDocument],
    max_proposals: usize,
) -> Vec<SwordSpiritProposal> {
    let lookup = document_lookup(documents);
    let mut proposals = Vec::new();
    for doc in documents {
        if proposals.len() >= max_proposals {
            break;
        }
        if doc.title.is_none() {
            proposals.push(metadata_title_proposal(doc));
            if proposals.len() >= max_proposals {
                break;
            }
        }
        for raw_link in &doc.wikilinks {
            if proposals.len() >= max_proposals {
                break;
            }
            let Some(target_key) = normalize_wikilink(raw_link) else {
                continue;
            };
            let Some(target_path) = lookup.get(&target_key).cloned() else {
                continue;
            };
            if target_path == doc.path {
                continue;
            }
            proposals.push(semantic_edge_proposal(doc, &target_path, raw_link));
        }
    }
    proposals
}

fn base_thinking_metadata(
    options: &SwordSpiritOptions,
    budget: &SwordSpiritBudget,
    candidate_limit: usize,
    mode: &str,
) -> SwordSpiritThinkingMetadata {
    SwordSpiritThinkingMetadata {
        mode: mode.to_string(),
        budget_profile: budget.profile.to_string(),
        trace_level: options.trace_level.as_str().to_string(),
        candidate_limit,
        lexical_per_source_cap: budget.lexical_per_source_cap,
        embedding_per_source_cap: budget.embedding_per_source_cap,
        reranker_per_source_cap: budget.reranker_per_source_cap,
        llm_candidate_cap: budget.llm_candidate_cap,
        llm_batch_size: budget.llm_batch_size,
        fallback_threshold: budget.fallback_threshold,
        fallback_policy: budget.fallback_policy.to_string(),
        embedding_provider: options.embedding_provider.clone(),
        embedding_model: options.embedding_model.clone(),
        embedding_dim: options.embedding_dim,
        embedding_invocation: "not_called".to_string(),
        embedded_count: 0,
        reranker_provider: options.reranker_provider.clone(),
        reranker_model: options.reranker_model.clone(),
        reranker_invocation: "not_called".to_string(),
        llm_invocation: "not_called".to_string(),
        candidate_count: 0,
        reranked_count: 0,
        accepted_count: 0,
        llm_calls: 0,
        rejected_count: 0,
        fallback_invocation: "not_used".to_string(),
        wall_time_ms: 0,
        note: String::new(),
    }
}

#[cfg(test)]
fn generate_neighbor_candidates(
    documents: &[SwordSpiritDocument],
    budget: &SwordSpiritBudget,
    embedding_provider: Option<&dyn EmbeddingProvider>,
) -> Result<Vec<SwordSpiritNeighborCandidate>> {
    generate_neighbor_candidates_with_limit(
        documents,
        budget,
        budget.candidate_limit(10),
        embedding_provider,
    )
}

fn generate_neighbor_candidates_with_limit(
    documents: &[SwordSpiritDocument],
    budget: &SwordSpiritBudget,
    candidate_limit: usize,
    embedding_provider: Option<&dyn EmbeddingProvider>,
) -> Result<Vec<SwordSpiritNeighborCandidate>> {
    let lookup = document_lookup(documents);
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    for doc in documents {
        for raw_link in &doc.wikilinks {
            let Some(target_key) = normalize_wikilink(raw_link) else {
                continue;
            };
            let Some(target_path) = lookup.get(&target_key).cloned() else {
                continue;
            };
            if target_path == doc.path {
                continue;
            }
            push_candidate(
                &mut candidates,
                &mut seen,
                doc,
                document_by_path(documents, &target_path),
                &target_path,
                "wikilink",
                0.78,
            );
        }
    }

    let features: Vec<(String, HashSet<String>)> = documents
        .iter()
        .map(|doc| (doc.path.clone(), document_tokens(doc)))
        .collect();
    let candidate_pools = large_corpus_candidate_pools(documents, &features);
    let mut by_source: HashMap<String, Vec<SwordSpiritNeighborCandidate>> = HashMap::new();
    for (i, source) in documents.iter().enumerate() {
        let target_indices: Vec<usize> = candidate_pools
            .as_ref()
            .map(|pools| pools[i].clone())
            .unwrap_or_else(|| (0..documents.len()).filter(|j| *j != i).collect());
        for j in target_indices {
            let target = &documents[j];
            if i == j {
                continue;
            }
            if !scopes_compatible(source, target) {
                continue;
            }
            let token_score = containment_score(&features[i].1, &features[j].1);
            let tag_score = tag_overlap_score(&source.tags, &target.tags);
            let lexical_score = (token_score * 0.72 + tag_score * 0.28).min(1.0);
            if lexical_score < 0.12 && tag_score <= 0.0 {
                continue;
            }
            let candidate = build_candidate(
                source,
                target,
                if tag_score > 0.0 {
                    "lexical+tag"
                } else {
                    "lexical"
                },
                lexical_score.max(0.18 * tag_score),
            );
            by_source
                .entry(source.path.clone())
                .or_default()
                .push(candidate);
        }
    }
    for (_, mut rows) in by_source {
        rows.sort_by(|a, b| {
            b.lexical_score
                .partial_cmp(&a.lexical_score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.target_path.cmp(&b.target_path))
        });
        for (rank, mut row) in rows
            .into_iter()
            .take(budget.lexical_per_source_cap.max(1))
            .enumerate()
        {
            if seen.insert((row.source_path.clone(), row.target_path.clone())) {
                let score = row.lexical_score;
                add_source_rank(&mut row, "lexical", rank + 1, score);
                refresh_candidate_scores(&mut row);
                candidates.push(row);
            }
        }
    }

    if let Some(provider) = embedding_provider {
        add_embedding_neighbors(
            documents,
            budget
                .embedding_per_source_cap
                .min(candidate_limit.max(1))
                .max(1),
            provider,
            &mut candidates,
            &mut seen,
            candidate_pools.as_deref(),
        )?;
    }

    sort_neighbors(&mut candidates);
    candidates.truncate(candidate_limit);
    Ok(candidates)
}

fn large_corpus_candidate_pools(
    documents: &[SwordSpiritDocument],
    features: &[(String, HashSet<String>)],
) -> Option<Vec<Vec<usize>>> {
    const LARGE_CORPUS_THRESHOLD: usize = 256;
    const TOKEN_CANDIDATE_CAP: usize = 96;
    const TAG_CANDIDATE_CAP: usize = 64;
    const TITLE_CANDIDATE_CAP: usize = 64;
    if documents.len() < LARGE_CORPUS_THRESHOLD {
        return None;
    }

    let mut token_index: HashMap<&str, Vec<usize>> = HashMap::new();
    for (idx, (_path, tokens)) in features.iter().enumerate() {
        for token in tokens {
            if useful_pool_token(token) {
                token_index.entry(token.as_str()).or_default().push(idx);
            }
        }
    }

    let mut tag_index: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, doc) in documents.iter().enumerate() {
        for tag in &doc.tags {
            let tag = tag.trim().to_ascii_lowercase();
            if !tag.is_empty() {
                tag_index.entry(tag).or_default().push(idx);
            }
        }
    }

    let mut title_index: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, doc) in documents.iter().enumerate() {
        for token in doc
            .title
            .as_deref()
            .map(lexical_tokens)
            .unwrap_or_default()
            .into_iter()
            .filter(|token| useful_pool_token(token))
        {
            title_index.entry(token).or_default().push(idx);
        }
    }

    let mut pools = Vec::with_capacity(documents.len());
    for (idx, doc) in documents.iter().enumerate() {
        let mut votes: HashMap<usize, usize> = HashMap::new();
        for token in &features[idx].1 {
            if !useful_pool_token(token) {
                continue;
            }
            if let Some(rows) = token_index.get(token.as_str()) {
                if rows.len() <= TOKEN_CANDIDATE_CAP {
                    for row in rows {
                        if *row != idx {
                            *votes.entry(*row).or_default() += 1;
                        }
                    }
                }
            }
        }
        for tag in &doc.tags {
            let tag = tag.trim().to_ascii_lowercase();
            if let Some(rows) = tag_index.get(&tag) {
                if rows.len() <= TAG_CANDIDATE_CAP {
                    for row in rows {
                        if *row != idx {
                            *votes.entry(*row).or_default() += 3;
                        }
                    }
                }
            }
        }
        if let Some(title) = doc.title.as_deref() {
            for token in lexical_tokens(title) {
                if !useful_pool_token(&token) {
                    continue;
                }
                if let Some(rows) = title_index.get(&token) {
                    if rows.len() <= TITLE_CANDIDATE_CAP {
                        for row in rows {
                            if *row != idx {
                                *votes.entry(*row).or_default() += 2;
                            }
                        }
                    }
                }
            }
        }
        let mut ranked: Vec<(usize, usize)> = votes.into_iter().collect();
        ranked.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| documents[a.0].path.cmp(&documents[b.0].path))
        });
        pools.push(ranked.into_iter().take(160).map(|(row, _)| row).collect());
    }
    Some(pools)
}

fn useful_pool_token(token: &str) -> bool {
    let len = token.chars().count();
    (3..=48).contains(&len) && !is_stopword(token)
}

fn add_embedding_neighbors(
    documents: &[SwordSpiritDocument],
    candidate_limit: usize,
    provider: &dyn EmbeddingProvider,
    candidates: &mut Vec<SwordSpiritNeighborCandidate>,
    seen: &mut HashSet<(String, String)>,
    candidate_pools: Option<&[Vec<usize>]>,
) -> Result<()> {
    if documents.len() < 2 {
        return Ok(());
    }
    let inputs: Vec<String> = documents
        .iter()
        .map(|doc| document_blurb(doc, 1400))
        .collect();
    let vectors = provider.embed_documents(&inputs)?;
    if vectors.len() != documents.len() {
        return Err(anyhow!(
            "Sword Spirit embedding response count mismatch: got {}, expected {}",
            vectors.len(),
            documents.len()
        ));
    }
    let profile = format!(
        "{}/{}@{}",
        provider.provider_id(),
        provider.model_id(),
        provider.dimension()
    );
    let per_source_limit = 8usize.min(candidate_limit.max(1));
    for (i, source) in documents.iter().enumerate() {
        let mut rows = Vec::new();
        let target_indices: Vec<usize> = candidate_pools
            .map(|pools| pools[i].clone())
            .unwrap_or_else(|| (0..documents.len()).filter(|j| *j != i).collect());
        for j in target_indices {
            if i == j || !scopes_compatible(source, &documents[j]) {
                continue;
            }
            let score = cosine_similarity(&vectors[i], &vectors[j]).clamp(0.0, 1.0);
            if score >= 0.18 {
                rows.push((j, score));
            }
        }
        rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        for (rank, (j, score)) in rows.into_iter().take(per_source_limit).enumerate() {
            upsert_embedding_candidate(
                candidates,
                seen,
                source,
                &documents[j],
                score,
                &profile,
                rank + 1,
            );
        }
    }
    Ok(())
}

fn upsert_embedding_candidate(
    candidates: &mut Vec<SwordSpiritNeighborCandidate>,
    seen: &mut HashSet<(String, String)>,
    source: &SwordSpiritDocument,
    target: &SwordSpiritDocument,
    embedding_score: f32,
    embedding_profile: &str,
    rank: usize,
) {
    let key = (source.path.clone(), target.path.clone());
    if !scopes_compatible(source, target) {
        return;
    }
    if seen.insert(key.clone()) {
        let mut candidate = build_candidate(source, target, "embedding", 0.0);
        candidate.embedding_score = Some(embedding_score);
        candidate.embedding_profile = Some(embedding_profile.to_string());
        add_source_rank(&mut candidate, "embedding", rank, embedding_score);
        refresh_candidate_scores(&mut candidate);
        candidates.push(candidate);
        return;
    }
    if let Some(candidate) = candidates
        .iter_mut()
        .find(|candidate| candidate.source_path == key.0 && candidate.target_path == key.1)
    {
        candidate.embedding_score = Some(embedding_score);
        candidate.embedding_profile = Some(embedding_profile.to_string());
        if !candidate.evidence_kind.contains("embedding") {
            candidate.evidence_kind = format!("{}+embedding", candidate.evidence_kind);
        }
        add_source_rank(candidate, "embedding", rank, embedding_score);
        refresh_candidate_scores(candidate);
    }
}

fn push_candidate(
    candidates: &mut Vec<SwordSpiritNeighborCandidate>,
    seen: &mut HashSet<(String, String)>,
    source: &SwordSpiritDocument,
    target: Option<&SwordSpiritDocument>,
    target_path: &str,
    evidence_kind: &str,
    lexical_score: f32,
) {
    if let Some(target_doc) = target {
        if !scopes_compatible(source, target_doc) {
            return;
        }
    }
    if !seen.insert((source.path.clone(), target_path.to_string())) {
        return;
    }
    let mut candidate = match target {
        Some(target_doc) => build_candidate(source, target_doc, evidence_kind, lexical_score),
        None => SwordSpiritNeighborCandidate {
            schema_version: "orderk.sword_spirit.neighbor_candidate.v1".to_string(),
            id: proposal_id("neighbor", &source.path, target_path, evidence_kind),
            source_path: source.path.clone(),
            target_path: target_path.to_string(),
            source_title: source.title.clone(),
            target_title: None,
            evidence_kind: evidence_kind.to_string(),
            lexical_score,
            embedding_score: None,
            embedding_profile: None,
            reranker_score: None,
            source_ranks: Vec::new(),
            rrf_score: 0.0,
            final_score: 0.0,
        },
    };
    add_source_rank(&mut candidate, evidence_kind, 1, lexical_score);
    refresh_candidate_scores(&mut candidate);
    candidates.push(candidate);
}

fn build_candidate(
    source: &SwordSpiritDocument,
    target: &SwordSpiritDocument,
    evidence_kind: &str,
    lexical_score: f32,
) -> SwordSpiritNeighborCandidate {
    let mut candidate = SwordSpiritNeighborCandidate {
        schema_version: "orderk.sword_spirit.neighbor_candidate.v1".to_string(),
        id: proposal_id("neighbor", &source.path, &target.path, evidence_kind),
        source_path: source.path.clone(),
        target_path: target.path.clone(),
        source_title: source.title.clone(),
        target_title: target.title.clone(),
        evidence_kind: evidence_kind.to_string(),
        lexical_score: lexical_score.clamp(0.0, 1.0),
        embedding_score: None,
        embedding_profile: None,
        reranker_score: None,
        source_ranks: Vec::new(),
        rrf_score: 0.0,
        final_score: 0.0,
    };
    if lexical_score > 0.0 {
        add_source_rank(&mut candidate, evidence_kind, 1, lexical_score);
    }
    refresh_candidate_scores(&mut candidate);
    candidate
}

fn rerank_neighbors(
    client: &SiliconFlowRerankerClient,
    documents: &[SwordSpiritDocument],
    neighbors: &mut [SwordSpiritNeighborCandidate],
    budget: &SwordSpiritBudget,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let doc_lookup: HashMap<&str, &SwordSpiritDocument> = documents
        .iter()
        .map(|doc| (doc.path.as_str(), doc))
        .collect();
    let mut by_source: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, candidate) in neighbors.iter().enumerate() {
        by_source
            .entry(candidate.source_path.clone())
            .or_default()
            .push(idx);
    }

    for (source_path, mut indexes) in by_source {
        indexes.sort_by(|a, b| {
            neighbors[*b]
                .lexical_score
                .partial_cmp(&neighbors[*a].lexical_score)
                .unwrap_or(Ordering::Equal)
        });
        indexes.truncate(budget.reranker_per_source_cap.max(1));
        let Some(source) = doc_lookup.get(source_path.as_str()).copied() else {
            continue;
        };
        let mut docs = Vec::new();
        let mut kept_indexes = Vec::new();
        for idx in indexes {
            if let Some(target) = doc_lookup.get(neighbors[idx].target_path.as_str()).copied() {
                docs.push(document_blurb(target, 900));
                kept_indexes.push(idx);
            }
        }
        if docs.is_empty() {
            continue;
        }
        let query = document_blurb(source, 900);
        let scores = client.rerank(&query, &docs)?;
        if scores.len() != kept_indexes.len() {
            warnings.push(format!(
                "reranker returned {} scores for {} candidates from {}; missing scores kept lexical-only",
                scores.len(),
                kept_indexes.len(),
                source_path
            ));
        }
        for (pos, score) in scores.into_iter().enumerate() {
            if let Some(idx) = kept_indexes.get(pos) {
                let score = score.clamp(0.0, 1.0);
                neighbors[*idx].reranker_score = Some(score);
                add_source_rank(&mut neighbors[*idx], "reranker", pos + 1, score);
                refresh_candidate_scores(&mut neighbors[*idx]);
            }
        }
    }
    Ok(())
}

fn should_use_reranker_fallback(llm_invocation: &str) -> bool {
    llm_invocation != "called"
}

fn proposals_from_decisions(
    decisions: &[ActiveProposalDecision],
    candidates: &[SwordSpiritNeighborCandidate],
    documents: &[SwordSpiritDocument],
    max_proposals: usize,
) -> (Vec<SwordSpiritProposal>, Vec<SwordSpiritRejectedDecision>) {
    let candidate_lookup: HashMap<&str, &SwordSpiritNeighborCandidate> = candidates
        .iter()
        .map(|candidate| (candidate.id.as_str(), candidate))
        .collect();
    let doc_lookup: HashMap<&str, &SwordSpiritDocument> = documents
        .iter()
        .map(|doc| (doc.path.as_str(), doc))
        .collect();
    let mut proposals = Vec::new();
    let mut rejected = Vec::new();
    for decision in decisions {
        let Some(candidate) = candidate_lookup
            .get(decision.candidate_id.as_str())
            .copied()
        else {
            rejected.push(rejected_decision(
                decision,
                None,
                "candidate_not_in_evidence_set",
                "LLM decision referenced a candidate id that was not present in the generated evidence set",
            ));
            continue;
        };
        if !decision.keep {
            rejected.push(rejected_decision(
                decision,
                Some(candidate),
                "llm_keep_false",
                "LLM explicitly rejected this candidate",
            ));
            continue;
        }
        let raw_relation = decision.relation.as_deref().unwrap_or("supports");
        if !is_allowed_relation(raw_relation) {
            rejected.push(rejected_decision(
                decision,
                Some(candidate),
                "relation_outside_prd_vocab",
                "LLM proposed a relation outside the PRD relation whitelist",
            ));
            continue;
        }
        let source = doc_lookup.get(candidate.source_path.as_str()).copied();
        let target = doc_lookup.get(candidate.target_path.as_str()).copied();
        if source.is_none() || target.is_none() {
            rejected.push(rejected_decision(
                decision,
                Some(candidate),
                "target_not_in_document_set",
                "Candidate source or target was not present in the scanned document set",
            ));
            continue;
        }
        if proposals.len() >= max_proposals {
            rejected.push(rejected_decision(
                decision,
                Some(candidate),
                "proposal_budget_exhausted",
                "Candidate was valid but exceeded the proposal budget cap",
            ));
            continue;
        }
        proposals.push(active_neighbor_proposal(
            candidate, decision, source, target,
        ));
    }
    (proposals, rejected)
}

fn rejected_decision(
    decision: &ActiveProposalDecision,
    candidate: Option<&SwordSpiritNeighborCandidate>,
    reason: &str,
    detail: &str,
) -> SwordSpiritRejectedDecision {
    SwordSpiritRejectedDecision {
        schema_version: "orderk.sword_spirit.rejected_decision.v1".to_string(),
        candidate_id: decision.candidate_id.clone(),
        source_path: candidate.map(|candidate| candidate.source_path.clone()),
        target_path: candidate.map(|candidate| candidate.target_path.clone()),
        reason: reason.to_string(),
        detail: detail.to_string(),
    }
}

fn active_neighbor_proposal(
    candidate: &SwordSpiritNeighborCandidate,
    decision: &ActiveProposalDecision,
    source: Option<&SwordSpiritDocument>,
    target: Option<&SwordSpiritDocument>,
) -> SwordSpiritProposal {
    let relation = normalize_relation(decision.relation.as_deref().unwrap_or("supports"));
    let model_confidence = decision
        .confidence
        .unwrap_or_else(|| candidate.reranker_score.unwrap_or(candidate.lexical_score))
        .clamp(0.0, 1.0);
    let confidence =
        ((model_confidence * 0.65) + (candidate_rank_score(candidate) * 0.35)).clamp(0.0, 1.0);
    let mut evidence = vec![
        SwordSpiritEvidence {
            path: candidate.source_path.clone(),
            kind: "active_candidate".to_string(),
            value: format!(
                "evidence_kind={}; lexical={:.4}; embedding={}; embedding_profile={}; reranker={}; rrf={:.6}; final={:.4}",
                candidate.evidence_kind,
                candidate.lexical_score,
                candidate
                    .embedding_score
                    .map(|score| format!("{score:.4}"))
                    .unwrap_or_else(|| "none".to_string()),
                candidate.embedding_profile.as_deref().unwrap_or("none"),
                candidate
                    .reranker_score
                    .map(|score| format!("{score:.4}"))
                    .unwrap_or_else(|| "none".to_string()),
                candidate.rrf_score,
                candidate.final_score
            ),
        },
        SwordSpiritEvidence {
            path: candidate.target_path.clone(),
            kind: "target".to_string(),
            value: target
                .and_then(|doc| doc.title.clone())
                .unwrap_or_else(|| candidate.target_path.clone()),
        },
    ];
    if let Some(source) = source {
        if !source.tags.is_empty() {
            evidence.push(SwordSpiritEvidence {
                path: source.path.clone(),
                kind: "source_tags".to_string(),
                value: source.tags.join(","),
            });
        }
    }
    if let Some(rationale) = decision
        .rationale
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        evidence.push(SwordSpiritEvidence {
            path: candidate.source_path.clone(),
            kind: "llm_rationale".to_string(),
            value: rationale.chars().take(500).collect(),
        });
    }
    SwordSpiritProposal {
        schema_version: "orderk.sword_spirit.proposal_active.v0".to_string(),
        id: proposal_id(
            "active_semantic_neighbor",
            &candidate.source_path,
            &candidate.target_path,
            &relation,
        ),
        proposal_type: "semantic_neighbor".to_string(),
        relation: Some(relation),
        source_path: candidate.source_path.clone(),
        target_path: Some(candidate.target_path.clone()),
        confidence,
        risk: "review".to_string(),
        auto_apply: false,
        human_review_required: true,
        evidence,
        rationale: decision
            .rationale
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                "MiniMax M3 kept this reranked neighbor as a candidate typed semantic edge."
                    .to_string()
            }),
    }
}

fn reranker_fallback_proposal(candidate: &SwordSpiritNeighborCandidate) -> SwordSpiritProposal {
    let confidence = candidate_rank_score(candidate).clamp(0.0, 1.0);
    SwordSpiritProposal {
        schema_version: "orderk.sword_spirit.proposal_active.v0".to_string(),
        id: proposal_id(
            "active_reranker_fallback",
            &candidate.source_path,
            &candidate.target_path,
            &candidate.evidence_kind,
        ),
        proposal_type: "semantic_neighbor".to_string(),
        relation: Some("supports".to_string()),
        source_path: candidate.source_path.clone(),
        target_path: Some(candidate.target_path.clone()),
        confidence,
        risk: "review".to_string(),
        auto_apply: false,
        human_review_required: true,
        evidence: vec![SwordSpiritEvidence {
            path: candidate.source_path.clone(),
            kind: "reranker_fallback".to_string(),
            value: format!(
                "lexical={:.4}; embedding={}; embedding_profile={}; reranker={}; rrf={:.6}; final={:.4}",
                candidate.lexical_score,
                candidate
                    .embedding_score
                    .map(|score| format!("{score:.4}"))
                    .unwrap_or_else(|| "none".to_string()),
                candidate
                    .embedding_profile
                    .as_deref()
                    .unwrap_or("none"),
                candidate
                    .reranker_score
                    .map(|score| format!("{score:.4}"))
                    .unwrap_or_else(|| "none".to_string()),
                candidate.rrf_score,
                candidate.final_score
            ),
        }],
        rationale: "MiniMax M3 was called but kept no candidate; this high-confidence reranker neighbor is preserved for human review rather than auto-application.".to_string(),
    }
}

impl SiliconFlowRerankerClient {
    fn from_env(model: &str) -> Result<Self> {
        Ok(Self {
            api_key: required_env_any(
                &[
                    "ORDERK_SWORD_RERANKER_API_KEY",
                    "HERMES_SILICONFLOW_API_KEY",
                    "ORDERK_SILICONFLOW_API_KEY",
                    "HINDSIGHT_API_RERANKER_SILICONFLOW_API_KEY",
                ],
                "SiliconFlow reranker",
            )?,
            model: model.trim().to_string(),
            base_url: env_string("ORDERK_SWORD_RERANKER_BASE_URL")
                .or_else(|| env_string("HINDSIGHT_API_RERANKER_SILICONFLOW_BASE_URL"))
                .unwrap_or_else(|| "https://api.siliconflow.cn/v1".to_string()),
        })
    }

    fn endpoint(&self) -> String {
        let trimmed = self.base_url.trim_end_matches('/');
        if trimmed.ends_with("/rerank") {
            trimmed.to_string()
        } else {
            format!("{trimmed}/rerank")
        }
    }

    fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<f32>> {
        if documents.is_empty() {
            return Ok(Vec::new());
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
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
        for attempt in 1..=3 {
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
                    if should_retry_http_status(code) && attempt < 3 {
                        last_error = message;
                        thread::sleep(Duration::from_millis(retry_backoff_ms(attempt)));
                        continue;
                    }
                    return Err(anyhow!("SiliconFlow reranker request failed: {message}"));
                }
                Err(ureq::Error::Transport(err)) => {
                    let message = err.to_string();
                    if attempt < 3 {
                        last_error = message;
                        thread::sleep(Duration::from_millis(retry_backoff_ms(attempt)));
                        continue;
                    }
                    return Err(anyhow!(
                        "SiliconFlow reranker request failed after 3 attempts: {message}"
                    ));
                }
            }
        }
        Err(anyhow!(
            "SiliconFlow reranker request failed after 3 attempts: {}",
            if last_error.is_empty() {
                "unknown error"
            } else {
                &last_error
            }
        ))
    }
}

impl AnthropicMiniMaxClient {
    fn from_env(model: &str) -> Result<Self> {
        Ok(Self {
            api_key: required_env_any(
                &[
                    "ORDERK_SWORD_LLM_API_KEY",
                    "HERMES_MINIMAX_API_KEY",
                    "HINDSIGHT_API_LLM_API_KEY",
                ],
                "Anthropic-compatible MiniMax M3",
            )?,
            model: model.trim().to_string(),
            base_url: env_string("ORDERK_SWORD_LLM_BASE_URL")
                .or_else(|| env_string("HINDSIGHT_API_LLM_BASE_URL"))
                .unwrap_or_else(|| "https://api.minimaxi.com/anthropic".to_string()),
            calls: 0,
        })
    }

    fn endpoint(&self) -> String {
        let trimmed = self.base_url.trim_end_matches('/');
        if trimmed.ends_with("/v1/messages") {
            trimmed.to_string()
        } else {
            format!("{trimmed}/v1/messages")
        }
    }

    fn decide_candidates(
        &mut self,
        candidates: &[SwordSpiritNeighborCandidate],
        documents: &[SwordSpiritDocument],
        max_proposals: usize,
        budget: &SwordSpiritBudget,
    ) -> Result<Vec<ActiveProposalDecision>> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let doc_lookup: HashMap<&str, &SwordSpiritDocument> = documents
            .iter()
            .map(|doc| (doc.path.as_str(), doc))
            .collect();
        let mut decisions: Vec<ActiveProposalDecision> = Vec::new();
        for chunk in candidates.chunks(budget.llm_batch_size.max(1)) {
            if decisions.iter().filter(|decision| decision.keep).count() >= max_proposals {
                break;
            }
            let payload_candidates: Vec<serde_json::Value> = chunk
                .iter()
                .map(|candidate| {
                    let source = doc_lookup.get(candidate.source_path.as_str()).copied();
                    let target = doc_lookup.get(candidate.target_path.as_str()).copied();
                    json!({
                        "candidate_id": candidate.id,
                        "source": source.map(candidate_doc_json).unwrap_or_else(|| json!({"path": candidate.source_path})),
                        "target": target.map(candidate_doc_json).unwrap_or_else(|| json!({"path": candidate.target_path})),
                        "evidence_kind": candidate.evidence_kind,
                        "lexical_score": candidate.lexical_score,
                        "embedding_score": candidate.embedding_score,
                        "embedding_profile": candidate.embedding_profile,
                        "reranker_score": candidate.reranker_score,
                    })
                })
                .collect();
            let prompt = format!(
                "你是 orderk V2 的剑灵（Sword Spirit）：只基于证据判断两个 Markdown 笔记是否应沉淀为 typed semantic edge。\n\n规则：\n1. 只保留对未来搜索/导航有帮助的关系；弱相关不要 keep。\n2. relation 必须只选 PRD P3 允许的 6 种之一：supports, refines, contradicts, replaces, depends_on, part_of。\n3. 不要提出改写原文，不要泄露或要求凭证。\n4. 返回 ONLY JSON array，不要代码块，不要解释。每项格式：{{\"candidate_id\":\"...\",\"keep\":true|false,\"relation\":\"supports\",\"confidence\":0.0-1.0,\"rationale\":\"一句中文理由\"}}。\n\n候选：{}",
                serde_json::to_string(&payload_candidates)?
            );
            let text = self.send_prompt(&prompt)?;
            let mut parsed = parse_decisions(&text)?;
            decisions.append(&mut parsed);
        }
        Ok(decisions)
    }

    fn send_prompt(&mut self, prompt: &str) -> Result<String> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(180))
            .build();
        let body = json!({
            "model": self.model,
            "max_tokens": 2600,
            "temperature": 0.1,
            "thinking": {"type": "disabled"},
            "system": "Return only the requested JSON. Do not include thinking, markdown, or explanation.",
            "messages": [{"role": "user", "content": prompt}]
        })
        .to_string();
        let mut last_error = String::new();
        for attempt in 1..=3 {
            self.calls += 1;
            match agent
                .post(&self.endpoint())
                .set("Content-Type", "application/json")
                .set("x-api-key", &self.api_key)
                .set("anthropic-version", "2023-06-01")
                .send_string(&body)
            {
                Ok(response) => {
                    let response_body = response.into_string().context("read LLM response")?;
                    return extract_anthropic_text(&response_body);
                }
                Err(ureq::Error::Status(code, response)) => {
                    let body = response.into_string().unwrap_or_default();
                    let message = format!("HTTP {}: {}", code, summarize_error_body(&body));
                    if should_retry_http_status(code) && attempt < 3 {
                        last_error = message;
                        thread::sleep(Duration::from_millis(retry_backoff_ms(attempt)));
                        continue;
                    }
                    return Err(anyhow!("MiniMax M3 request failed: {message}"));
                }
                Err(ureq::Error::Transport(err)) => {
                    let message = err.to_string();
                    if attempt < 3 {
                        last_error = message;
                        thread::sleep(Duration::from_millis(retry_backoff_ms(attempt)));
                        continue;
                    }
                    return Err(anyhow!(
                        "MiniMax M3 request failed after 3 attempts: {message}"
                    ));
                }
            }
        }
        Err(anyhow!(
            "MiniMax M3 request failed after 3 attempts: {}",
            if last_error.is_empty() {
                "unknown error"
            } else {
                &last_error
            }
        ))
    }
}

fn parse_decisions(text: &str) -> Result<Vec<ActiveProposalDecision>> {
    let json_text = extract_json_fragment(text)
        .ok_or_else(|| anyhow!("LLM response did not contain a JSON array or object"))?;
    if let Ok(items) = serde_json::from_str::<Vec<ActiveProposalDecision>>(json_text) {
        return Ok(items);
    }
    if let Ok(item) = serde_json::from_str::<ActiveProposalDecision>(json_text) {
        return Ok(vec![item]);
    }
    #[derive(Debug, Deserialize)]
    struct Envelope {
        decisions: Vec<ActiveProposalDecision>,
    }
    let envelope: Envelope = serde_json::from_str(json_text).with_context(|| {
        format!(
            "parse LLM decision JSON; response preview: {}",
            text.chars().take(300).collect::<String>()
        )
    })?;
    Ok(envelope.decisions)
}

fn extract_anthropic_text(body: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(body).context("parse LLM response JSON")?;
    let mut out = String::new();
    if let Some(blocks) = value.get("content").and_then(|v| v.as_array()) {
        for block in blocks {
            if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(text);
                }
            }
        }
    }
    if out.trim().is_empty() {
        return Err(anyhow!(
            "LLM response had no text content; response preview: {}",
            body.chars().take(300).collect::<String>()
        ));
    }
    Ok(out)
}

fn extract_json_fragment(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return Some(trimmed);
    }
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed);
    }
    if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']')) {
        if start < end {
            return Some(&trimmed[start..=end]);
        }
    }
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end {
            return Some(&trimmed[start..=end]);
        }
    }
    None
}

fn candidate_doc_json(doc: &SwordSpiritDocument) -> serde_json::Value {
    json!({
        "path": doc.path,
        "title": doc.title,
        "tags": doc.tags,
        "excerpt": compact_excerpt(&doc.body_excerpt, 420),
    })
}

fn document_blurb(doc: &SwordSpiritDocument, limit: usize) -> String {
    compact_excerpt(
        &format!(
            "path: {}\ntitle: {}\ntags: {}\n{}",
            doc.path,
            doc.title.as_deref().unwrap_or(""),
            doc.tags.join(", "),
            doc.body_excerpt
        ),
        limit,
    )
}

fn document_tokens(doc: &SwordSpiritDocument) -> HashSet<String> {
    lexical_tokens(&format!(
        "{} {} {} {}",
        doc.path,
        doc.title.as_deref().unwrap_or(""),
        doc.tags.join(" "),
        doc.body_excerpt
    ))
}

fn lexical_tokens(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            current.push(ch.to_ascii_lowercase());
        } else {
            push_ascii_token(&mut out, &mut current);
        }
    }
    push_ascii_token(&mut out, &mut current);

    let cjkish: Vec<char> = text
        .chars()
        .filter(|ch| !ch.is_ascii() && ch.is_alphanumeric())
        .collect();
    for window in cjkish.windows(2) {
        out.insert(window.iter().collect::<String>());
    }
    for ch in cjkish {
        out.insert(ch.to_string());
    }
    out
}

fn push_ascii_token(out: &mut HashSet<String>, current: &mut String) {
    if current.chars().count() >= 2 && !is_stopword(current) {
        out.insert(current.clone());
    }
    current.clear();
}

fn is_stopword(value: &str) -> bool {
    matches!(
        value,
        "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "this"
            | "that"
            | "you"
            | "are"
            | "was"
            | "were"
            | "not"
            | "but"
            | "into"
            | "about"
            | "have"
            | "has"
            | "will"
            | "can"
            | "use"
            | "uses"
            | "using"
            | "see"
            | "note"
            | "notes"
    )
}

fn containment_score(left: &HashSet<String>, right: &HashSet<String>) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let overlap = left.intersection(right).count() as f32;
    let denom = left.len().min(right.len()).max(1) as f32;
    (overlap / denom).min(1.0)
}

fn tag_overlap_score(left: &[String], right: &[String]) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let left_set: HashSet<String> = left.iter().map(|s| s.to_ascii_lowercase()).collect();
    let right_set: HashSet<String> = right.iter().map(|s| s.to_ascii_lowercase()).collect();
    containment_score(&left_set, &right_set)
}

fn candidate_rank_score(candidate: &SwordSpiritNeighborCandidate) -> f32 {
    let base_score = candidate
        .embedding_score
        .map(|score| (score * 0.72) + (candidate.lexical_score * 0.28))
        .unwrap_or(candidate.lexical_score);
    match candidate.reranker_score {
        Some(score) => (score * 0.82) + (base_score * 0.18),
        None => base_score,
    }
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || left.len() != right.len() {
        return 0.0;
    }
    let dot = left
        .iter()
        .zip(right.iter())
        .map(|(l, r)| l * r)
        .sum::<f32>();
    let left_norm = left.iter().map(|v| v * v).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|v| v * v).sum::<f32>().sqrt();
    if left_norm <= f32::EPSILON || right_norm <= f32::EPSILON {
        0.0
    } else {
        dot / (left_norm * right_norm)
    }
}

fn sort_neighbors(neighbors: &mut [SwordSpiritNeighborCandidate]) {
    neighbors.sort_by(|a, b| {
        candidate_rank_score(b)
            .partial_cmp(&candidate_rank_score(a))
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.source_path.cmp(&b.source_path))
            .then_with(|| a.target_path.cmp(&b.target_path))
            .then_with(|| a.id.cmp(&b.id))
    });
}

fn add_source_rank(
    candidate: &mut SwordSpiritNeighborCandidate,
    source: &str,
    rank: usize,
    score: f32,
) {
    let source = source.to_ascii_lowercase().replace('+', "_");
    let rank = rank.max(1);
    let score = score.clamp(0.0, 1.0);
    candidate.source_ranks.retain(|row| row.source != source);
    candidate.source_ranks.push(SwordSpiritSourceRank {
        source,
        rank,
        score,
        rrf_score: reciprocal_rank_score(rank),
    });
    candidate.source_ranks.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| a.rank.cmp(&b.rank))
            .then_with(|| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal))
    });
}

fn reciprocal_rank_score(rank: usize) -> f32 {
    1.0 / (60.0 + rank.max(1) as f32)
}

fn refresh_candidate_scores(candidate: &mut SwordSpiritNeighborCandidate) {
    candidate.rrf_score = candidate
        .source_ranks
        .iter()
        .map(|row| row.rrf_score)
        .sum::<f32>();
    let fused = (candidate.rrf_score * 12.0).clamp(0.0, 1.0);
    candidate.final_score =
        ((candidate_rank_score(candidate) * 0.74) + (fused * 0.26)).clamp(0.0, 1.0);
}

fn is_allowed_relation(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "supports" | "refines" | "contradicts" | "replaces" | "depends_on" | "part_of"
    )
}

fn scopes_compatible(left: &SwordSpiritDocument, right: &SwordSpiritDocument) -> bool {
    let left_scope = document_scope(left);
    let right_scope = document_scope(right);
    match (left_scope, right_scope) {
        (Some(left_scope), Some(right_scope)) => left_scope == right_scope,
        _ => true,
    }
}

fn document_scope(doc: &SwordSpiritDocument) -> Option<String> {
    for tag in &doc.tags {
        let lower = tag.trim().trim_start_matches('#').to_ascii_lowercase();
        if lower.starts_with("project:")
            || lower.starts_with("project/")
            || lower.starts_with("scope:")
        {
            return Some(lower.replace('/', ":"));
        }
    }
    let mut parts = doc.path.split('/');
    let first = parts.next()?.to_ascii_lowercase();
    if parts.next().is_some() && (first.starts_with("project-") || first.starts_with("project_")) {
        return Some(first);
    }
    None
}

fn document_by_path<'a>(
    documents: &'a [SwordSpiritDocument],
    path: &str,
) -> Option<&'a SwordSpiritDocument> {
    documents.iter().find(|doc| doc.path == path)
}

fn document_lookup(documents: &[SwordSpiritDocument]) -> HashMap<String, String> {
    let mut lookup = HashMap::new();
    for doc in documents {
        let path = doc.path.clone();
        insert_key(&mut lookup, &path, &path);
        if let Some(stripped) = path.strip_suffix(".md") {
            insert_key(&mut lookup, stripped, &path);
        }
        if let Some(stem) = Path::new(&path).file_stem().and_then(|s| s.to_str()) {
            insert_key(&mut lookup, stem, &path);
        }
        if let Some(title) = &doc.title {
            insert_key(&mut lookup, title, &path);
        }
    }
    lookup
}

fn insert_key(lookup: &mut HashMap<String, String>, key: &str, path: &str) {
    let normalized = normalize_lookup_key(key);
    if !normalized.is_empty() {
        lookup.entry(normalized).or_insert_with(|| path.to_string());
    }
}

fn normalize_wikilink(raw: &str) -> Option<String> {
    let before_alias = raw.split('|').next().unwrap_or(raw);
    let before_anchor = before_alias.split('#').next().unwrap_or(before_alias);
    let normalized = normalize_lookup_key(before_anchor);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn normalize_lookup_key(raw: &str) -> String {
    raw.trim()
        .trim_matches('/')
        .trim_end_matches(".md")
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn normalize_relation(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "supports" | "support" | "supported_by" | "related_to" | "related" | "mentions"
        | "mention" => "supports".to_string(),
        "refines" | "refine" | "duplicates" | "duplicate" | "same_as" => "refines".to_string(),
        "contradicts" | "contradict" | "contrasts_with" | "contrasts-with" | "contrast" => {
            "contradicts".to_string()
        }
        "replaces" | "replace" | "supersedes" | "supersede" => "replaces".to_string(),
        "depends_on" | "depends-on" | "dependency" | "implements" | "implementation" => {
            "depends_on".to_string()
        }
        "part_of" | "part-of" | "contains" | "component_of" => "part_of".to_string(),
        _ => "supports".to_string(),
    }
}

fn metadata_title_proposal(doc: &SwordSpiritDocument) -> SwordSpiritProposal {
    let evidence = SwordSpiritEvidence {
        path: doc.path.clone(),
        kind: "missing_h1_title".to_string(),
        value: "document has no first-level Markdown heading".to_string(),
    };
    let id = proposal_id("metadata_title", &doc.path, "", &doc.hash);
    SwordSpiritProposal {
        schema_version: "orderk.sword_spirit.proposal_mvp.v0".to_string(),
        id,
        proposal_type: "metadata_backfill".to_string(),
        relation: None,
        source_path: doc.path.clone(),
        target_path: None,
        confidence: 0.6,
        risk: "low".to_string(),
        auto_apply: false,
        human_review_required: true,
        evidence: vec![evidence],
        rationale: "A visible H1 title would improve human Markdown-base navigation and orderk result labels; heuristic mode only proposes it.".to_string(),
    }
}

fn semantic_edge_proposal(
    doc: &SwordSpiritDocument,
    target_path: &str,
    raw_link: &str,
) -> SwordSpiritProposal {
    let id = proposal_id("semantic_edge", &doc.path, target_path, raw_link);
    let mut evidence = vec![SwordSpiritEvidence {
        path: doc.path.clone(),
        kind: "wikilink".to_string(),
        value: raw_link.to_string(),
    }];
    if !doc.tags.is_empty() {
        evidence.push(SwordSpiritEvidence {
            path: doc.path.clone(),
            kind: "source_tags".to_string(),
            value: doc.tags.join(","),
        });
    }
    SwordSpiritProposal {
        schema_version: "orderk.sword_spirit.proposal_mvp.v0".to_string(),
        id,
        proposal_type: "semantic_edge".to_string(),
        relation: Some("depends_on".to_string()),
        source_path: doc.path.clone(),
        target_path: Some(target_path.to_string()),
        confidence: 0.42,
        risk: "review".to_string(),
        auto_apply: false,
        human_review_required: true,
        evidence,
        rationale: "Existing wikilink is treated as evidence for a candidate typed semantic edge; relation is proposal-only until accepted by audit.".to_string(),
    }
}

fn proposal_id(kind: &str, source: &str, target: &str, extra: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    hasher.update(b"\0");
    hasher.update(source.as_bytes());
    hasher.update(b"\0");
    hasher.update(target.as_bytes());
    hasher.update(b"\0");
    hasher.update(extra.as_bytes());
    let hash = hex::encode(hasher.finalize());
    format!("ss-{}", &hash[..16])
}

fn create_unique_run_dir(sidecar_root: &Path) -> Result<(String, PathBuf)> {
    let runs_dir = sidecar_root.join("runs");
    fs::create_dir_all(&runs_dir)
        .with_context(|| format!("create sword-spirit runs dir: {}", runs_dir.display()))?;
    for attempt in 0..100u32 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let run_id = format!(
            "sword-{}-{}-{}-{}",
            Utc::now().format("%Y%m%dT%H%M%SZ"),
            std::process::id(),
            nanos,
            attempt
        );
        let run_dir = runs_dir.join(&run_id);
        match fs::create_dir(&run_dir) {
            Ok(()) => return Ok((run_id, run_dir)),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("create sword-spirit run dir: {}", run_dir.display()))
            }
        }
    }
    anyhow::bail!(
        "could not allocate a unique sword-spirit run dir under {}",
        runs_dir.display()
    );
}

fn write_json_pretty<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(path, bytes).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn write_jsonl<T: Serialize>(path: &Path, rows: &[T]) -> Result<()> {
    let mut file = fs::File::create(path).with_context(|| format!("write {}", path.display()))?;
    for row in rows {
        serde_json::to_writer(&mut file, row)?;
        file.write_all(b"\n")?;
    }
    Ok(())
}

fn write_report(
    path: &Path,
    run_id: &str,
    vault: &Path,
    proposals: &[SwordSpiritProposal],
    neighbors: &[SwordSpiritNeighborCandidate],
    options: &SwordSpiritOptions,
    thinking: &SwordSpiritThinkingMetadata,
) -> Result<()> {
    let mut body = String::new();
    body.push_str("# orderk Sword Spirit Report\n\n");
    body.push_str(&format!("- run_id: `{run_id}`\n"));
    body.push_str(&format!("- vault: `{}`\n", vault.display()));
    body.push_str(&format!("- mode: `{}`\n", options.thinking_mode.as_str()));
    body.push_str(&format!(
        "- llm: `{}` / `{}` ({})\n",
        options.llm_provider, options.llm_model, thinking.llm_invocation
    ));
    body.push_str(&format!(
        "- embedding: `{}` / `{}` dim {} ({}, embedded {})\n",
        options.embedding_provider,
        options.embedding_model,
        options.embedding_dim,
        thinking.embedding_invocation,
        thinking.embedded_count
    ));
    body.push_str(&format!(
        "- reranker: `{}` / `{}` ({})\n",
        options.reranker_provider, options.reranker_model, thinking.reranker_invocation
    ));
    body.push_str(&format!(
        "- candidates: {} / reranked: {} / accepted: {} / llm_calls: {}\n",
        thinking.candidate_count,
        thinking.reranked_count,
        thinking.accepted_count,
        thinking.llm_calls
    ));
    body.push_str("- boundary: external Markdown base remains the substrate; orderk writes only `.orderk/sword_spirit/` sidecar artifacts.\n\n");
    body.push_str(&format!("## Proposals ({})\n\n", proposals.len()));
    for proposal in proposals.iter().take(20) {
        body.push_str(&format!(
            "- `{}` `{}` {} -> {} ({}, confidence {:.2})\n",
            proposal.id,
            proposal.proposal_type,
            proposal.source_path,
            proposal.target_path.as_deref().unwrap_or("<metadata>"),
            proposal.relation.as_deref().unwrap_or("n/a"),
            proposal.confidence
        ));
    }
    if proposals.len() > 20 {
        body.push_str(&format!(
            "- ... {} more proposals in proposals.jsonl\n",
            proposals.len() - 20
        ));
    }
    body.push_str(&format!("\n## Top neighbors ({})\n\n", neighbors.len()));
    for neighbor in neighbors.iter().take(20) {
        body.push_str(&format!(
            "- `{}` {} -> {} kind {} lexical {:.3} embedding {} profile {} reranker {}\n",
            neighbor.id,
            neighbor.source_path,
            neighbor.target_path,
            neighbor.evidence_kind,
            neighbor.lexical_score,
            neighbor
                .embedding_score
                .map(|score| format!("{score:.3}"))
                .unwrap_or_else(|| "n/a".to_string()),
            neighbor.embedding_profile.as_deref().unwrap_or("n/a"),
            neighbor
                .reranker_score
                .map(|score| format!("{score:.3}"))
                .unwrap_or_else(|| "n/a".to_string())
        ));
    }
    fs::write(path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn sword_spirit_boundary() -> SwordSpiritBoundary {
    SwordSpiritBoundary {
        markdown_base_owner: "external Markdown base / adapter".to_string(),
        orderk_role: "intelligence layer: index/search/semantic proposals/eval over plain Markdown"
            .to_string(),
        writes: vec![
            ".orderk/sword_spirit/runs/<run_id>/input-manifest.json".to_string(),
            ".orderk/sword_spirit/runs/<run_id>/neighbors.jsonl".to_string(),
            ".orderk/sword_spirit/runs/<run_id>/proposals.jsonl".to_string(),
            ".orderk/sword_spirit/runs/<run_id>/rejected.jsonl".to_string(),
            ".orderk/sword_spirit/runs/<run_id>/audit.jsonl".to_string(),
            ".orderk/sword_spirit/runs/<run_id>/report.md".to_string(),
        ],
        forbidden: vec![
            "mutating raw/source Markdown during proposal runs".to_string(),
            "reimplementing vault/editor/visual graph base inside orderk".to_string(),
            "default query-time reflection".to_string(),
            "secret logging or LLM key capture".to_string(),
        ],
    }
}

fn should_retry_http_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

fn retry_backoff_ms(attempt: usize) -> u64 {
    500 * 2_u64.saturating_pow((attempt.saturating_sub(1)) as u32)
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

fn compact_excerpt(text: &str, max_chars: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = compact.chars().take(max_chars).collect();
    if out.len() < compact.len() {
        out.push('…');
    }
    out
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value != "***" && !value.contains("<redacted"))
}

fn required_env_any(names: &[&str], label: &str) -> Result<String> {
    for name in names {
        if let Some(value) = env_string(name) {
            return Ok(value);
        }
    }
    Err(anyhow!(
        "{label} API key is missing; set one of {}",
        names.join(" or ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::scan_vault;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "orderk-{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sword_spirit_generates_sidecar_proposals_without_mutating_markdown() {
        let vault = temp_dir("sword-spirit");
        fs::write(
            vault.join("alpha.md"),
            "---\ntags: [project, orderk]\n---\n# Alpha\nSee [[Bravo]] for the base boundary.\n",
        )
        .unwrap();
        fs::write(
            vault.join("bravo.md"),
            "# Bravo\nMarkdown base stays external.\n",
        )
        .unwrap();
        fs::create_dir_all(vault.join(".orderk")).unwrap();
        fs::write(vault.join(".orderk/ignored.md"), "# ignored\n").unwrap();
        let before = fs::read_to_string(vault.join("alpha.md")).unwrap();

        let response = run_sword_spirit(
            &vault,
            &SwordSpiritOptions {
                llm_provider: "anthropic".to_string(),
                llm_model: "MiniMax-M3".to_string(),
                ..SwordSpiritOptions::default()
            },
        )
        .unwrap();

        assert!(response.ok);
        assert_eq!(response.schema_version, "orderk.sword_spirit.run.v1");
        assert_eq!(response.mode, "heuristic");
        assert_eq!(response.files_scanned, 2, ".orderk sidecar must be ignored");
        assert_eq!(fs::read_to_string(vault.join("alpha.md")).unwrap(), before);
        assert!(Path::new(&response.proposals_path).exists());
        assert!(Path::new(&response.audit_path).exists());
        assert!(Path::new(&response.report_path).exists());
        assert!(Path::new(&response.neighbors_path).exists());
        assert!(response.proposals.iter().any(|proposal| {
            proposal.proposal_type == "semantic_edge"
                && proposal.relation.as_deref() == Some("depends_on")
                && proposal.source_path == "alpha.md"
                && proposal.target_path.as_deref() == Some("bravo.md")
        }));
        let scanned = scan_vault(&vault).unwrap();
        assert_eq!(
            scanned.len(),
            2,
            "generated report.md must not be re-scanned"
        );
        let _ = fs::remove_dir_all(vault);
    }

    #[test]
    fn sword_spirit_status_reports_latest_run() {
        let vault = temp_dir("sword-status");
        fs::write(vault.join("alpha.md"), "# Alpha\n").unwrap();
        let run = run_sword_spirit(&vault, &SwordSpiritOptions::default()).unwrap();
        let status = sword_spirit_status(&vault).unwrap();
        assert!(status.ok);
        assert_eq!(status.runs, 1);
        assert_eq!(status.latest_run_id.as_deref(), Some(run.run_id.as_str()));
        let _ = fs::remove_dir_all(vault);
    }

    #[test]
    fn sword_spirit_run_ids_do_not_collide_in_same_process() {
        let vault = temp_dir("sword-run-id");
        fs::write(vault.join("alpha.md"), "# Alpha\nSee [[Bravo]].\n").unwrap();
        fs::write(vault.join("bravo.md"), "# Bravo\n").unwrap();

        let first = run_sword_spirit(&vault, &SwordSpiritOptions::default()).unwrap();
        let second = run_sword_spirit(&vault, &SwordSpiritOptions::default()).unwrap();
        let status = sword_spirit_status(&vault).unwrap();

        assert_ne!(first.run_id, second.run_id);
        assert_ne!(first.run_dir, second.run_dir);
        assert!(Path::new(&first.audit_path).exists());
        assert!(Path::new(&second.audit_path).exists());
        assert_eq!(status.runs, 2);
        let _ = fs::remove_dir_all(vault);
    }

    #[test]
    fn sword_spirit_relation_normalization_stays_inside_prd_vocab() {
        let allowed = [
            "supports",
            "refines",
            "contradicts",
            "replaces",
            "depends_on",
            "part_of",
        ];
        for raw in [
            "related_to",
            "mentions",
            "duplicates",
            "supersedes",
            "implements",
            "contrasts_with",
            "unknown_relation",
        ] {
            let normalized = normalize_relation(raw);
            assert!(
                allowed.contains(&normalized.as_str()),
                "{raw} normalized outside PRD vocab: {normalized}"
            );
        }
    }

    #[test]
    fn sword_spirit_active_candidate_generation_handles_cjk_and_tags() {
        let docs = vec![
            SwordSpiritDocument {
                path: "a.md".to_string(),
                hash: "a".to_string(),
                title: Some("剑灵 主动思考".to_string()),
                wikilinks: Vec::new(),
                tags: vec!["orderk".to_string()],
                body_excerpt: "Hindsight 对比 检索 语义边".to_string(),
            },
            SwordSpiritDocument {
                path: "b.md".to_string(),
                hash: "b".to_string(),
                title: Some("Hindsight 检索机制".to_string()),
                wikilinks: Vec::new(),
                tags: vec!["orderk".to_string()],
                body_excerpt: "语义边 主动沉淀 搜索效果".to_string(),
            },
        ];
        let budget = SwordSpiritBudgetProfile::DigestStandard.budget();
        let candidates = generate_neighbor_candidates(&docs, &budget, None).unwrap();
        assert!(candidates
            .iter()
            .any(|c| c.source_path == "a.md" && c.target_path == "b.md"));
    }

    #[test]
    fn sword_spirit_embedding_neighbors_are_materialized_for_digest_candidates() {
        let docs = vec![
            SwordSpiritDocument {
                path: "sword.md".to_string(),
                hash: "a".to_string(),
                title: Some("Sword Spirit".to_string()),
                wikilinks: Vec::new(),
                tags: vec!["orderk".to_string()],
                body_excerpt: "Sword Spirit digest uses embedding neighbors before reranker and LLM proposals.".to_string(),
            },
            SwordSpiritDocument {
                path: "hindsight.md".to_string(),
                hash: "b".to_string(),
                title: Some("Hindsight Stack".to_string()),
                wikilinks: Vec::new(),
                tags: vec!["retrieval".to_string()],
                body_excerpt: "Hindsight retrieval uses embedding recall with reranking and structured memory.".to_string(),
            },
        ];
        let provider = crate::embedding::MockEmbeddingProvider::new(32);
        let budget = SwordSpiritBudgetProfile::DigestStandard.budget();

        let candidates = generate_neighbor_candidates(&docs, &budget, Some(&provider)).unwrap();

        assert!(
            candidates.iter().any(|candidate| {
                candidate.evidence_kind.contains("embedding")
                    && candidate.embedding_score.is_some()
                    && candidate.embedding_profile.as_deref() == Some("mock/mock-32@32")
            }),
            "expected at least one persisted embedding-backed neighbor, got {candidates:#?}"
        );
    }

    #[test]
    fn sword_spirit_budget_profiles_drive_active_caps_and_fallback_thresholds() {
        let low = SwordSpiritBudgetProfile::parse("digest_low")
            .unwrap()
            .budget();
        let standard = SwordSpiritBudgetProfile::parse("digest-standard")
            .unwrap()
            .budget();
        let deep = SwordSpiritBudgetProfile::parse("digest_deep")
            .unwrap()
            .budget();
        let eval = SwordSpiritBudgetProfile::parse("eval").unwrap().budget();

        assert_eq!(low.profile, "digest_low");
        assert!(low.candidate_limit(10) < standard.candidate_limit(10));
        assert!(standard.candidate_limit(10) < deep.candidate_limit(10));
        assert!(eval.candidate_limit(10) >= deep.candidate_limit(10));
        assert!(low.llm_candidate_cap < standard.llm_candidate_cap);
        assert!(standard.llm_candidate_cap <= deep.llm_candidate_cap);
        assert!(low.fallback_threshold > deep.fallback_threshold);
    }

    #[test]
    fn sword_spirit_heuristic_runs_report_budget_and_trace_contract() {
        let vault = temp_dir("sword-budget-trace");
        fs::write(vault.join("alpha.md"), "# Alpha\nSee [[Bravo]].\n").unwrap();
        fs::write(vault.join("bravo.md"), "# Bravo\n").unwrap();

        let response = run_sword_spirit(
            &vault,
            &SwordSpiritOptions {
                budget_profile: SwordSpiritBudgetProfile::DigestLow,
                trace_level: SwordSpiritTraceLevel::Compact,
                ..SwordSpiritOptions::default()
            },
        )
        .unwrap();

        assert_eq!(response.thinking.budget_profile, "digest_low");
        assert_eq!(response.thinking.trace_level, "compact");
        assert_eq!(response.thinking.candidate_limit, 24);
        assert_eq!(response.thinking.fallback_policy, "proposal_only_review");
        assert!(response.thinking.fallback_threshold > 0.0);
        assert!(response.thinking.note.contains("budget_profile=digest_low"));
        let _ = fs::remove_dir_all(vault);
    }

    #[test]
    fn sword_spirit_candidates_keep_rrf_source_ranks_and_score_trace() {
        let docs = vec![
            SwordSpiritDocument {
                path: "project/a.md".to_string(),
                hash: "a".to_string(),
                title: Some("Alpha Retrieval".to_string()),
                wikilinks: Vec::new(),
                tags: vec!["project:race".to_string()],
                body_excerpt: "embedding reranker sword spirit trace evidence".to_string(),
            },
            SwordSpiritDocument {
                path: "project/b.md".to_string(),
                hash: "b".to_string(),
                title: Some("Bravo Retrieval".to_string()),
                wikilinks: Vec::new(),
                tags: vec!["project:race".to_string()],
                body_excerpt: "embedding reranker sword spirit trace evidence".to_string(),
            },
        ];
        let provider = crate::embedding::MockEmbeddingProvider::new(16);
        let budget = SwordSpiritBudgetProfile::DigestLow.budget();

        let candidates = generate_neighbor_candidates(&docs, &budget, Some(&provider)).unwrap();

        let candidate = candidates
            .iter()
            .find(|candidate| {
                candidate.source_path == "project/a.md" && candidate.target_path == "project/b.md"
            })
            .expect("expected a->b candidate");
        assert!(
            candidate.rrf_score > 0.0,
            "candidate should have RRF score: {candidate:#?}"
        );
        assert!(
            candidate.final_score > 0.0,
            "candidate should have final score: {candidate:#?}"
        );
        assert!(
            candidate
                .source_ranks
                .iter()
                .any(|rank| rank.source == "lexical")
                || candidate
                    .source_ranks
                    .iter()
                    .any(|rank| rank.source == "embedding"),
            "candidate should preserve rank-source trace: {candidate:#?}"
        );
    }

    #[test]
    fn sword_spirit_evidence_gate_rejects_unknown_candidate_and_invalid_relation() {
        let docs = vec![
            SwordSpiritDocument {
                path: "scope/a.md".to_string(),
                hash: "a".to_string(),
                title: Some("A".to_string()),
                wikilinks: Vec::new(),
                tags: vec!["project:race".to_string()],
                body_excerpt: "alpha".to_string(),
            },
            SwordSpiritDocument {
                path: "scope/b.md".to_string(),
                hash: "b".to_string(),
                title: Some("B".to_string()),
                wikilinks: Vec::new(),
                tags: vec!["project:race".to_string()],
                body_excerpt: "bravo".to_string(),
            },
        ];
        let mut candidate = build_candidate(&docs[0], &docs[1], "lexical", 0.8);
        candidate.source_ranks.push(SwordSpiritSourceRank {
            source: "lexical".to_string(),
            rank: 1,
            score: 0.8,
            rrf_score: reciprocal_rank_score(1),
        });
        refresh_candidate_scores(&mut candidate);
        let decisions = vec![
            ActiveProposalDecision {
                candidate_id: "missing-candidate".to_string(),
                keep: true,
                relation: Some("supports".to_string()),
                confidence: Some(0.9),
                rationale: Some("unknown target".to_string()),
            },
            ActiveProposalDecision {
                candidate_id: candidate.id.clone(),
                keep: true,
                relation: Some("made_up_relation".to_string()),
                confidence: Some(0.9),
                rationale: Some("bad relation".to_string()),
            },
            ActiveProposalDecision {
                candidate_id: candidate.id.clone(),
                keep: true,
                relation: Some("supports".to_string()),
                confidence: Some(0.9),
                rationale: Some("valid relation".to_string()),
            },
        ];

        let (proposals, rejected) = proposals_from_decisions(&decisions, &[candidate], &docs, 10);

        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].relation.as_deref(), Some("supports"));
        assert_eq!(rejected.len(), 2);
        assert!(rejected
            .iter()
            .any(|row| row.reason == "candidate_not_in_evidence_set"));
        assert!(rejected
            .iter()
            .any(|row| row.reason == "relation_outside_prd_vocab"));
    }

    #[test]
    fn sword_spirit_scope_isolation_blocks_cross_project_candidates() {
        let docs = vec![
            SwordSpiritDocument {
                path: "project-a/same.md".to_string(),
                hash: "a".to_string(),
                title: Some("Shared Concept".to_string()),
                wikilinks: Vec::new(),
                tags: vec!["project:alpha".to_string()],
                body_excerpt: "shared concept retrieval sword spirit".to_string(),
            },
            SwordSpiritDocument {
                path: "project-b/same.md".to_string(),
                hash: "b".to_string(),
                title: Some("Shared Concept".to_string()),
                wikilinks: Vec::new(),
                tags: vec!["project:beta".to_string()],
                body_excerpt: "shared concept retrieval sword spirit".to_string(),
            },
        ];
        let budget = SwordSpiritBudgetProfile::DigestLow.budget();

        let candidates = generate_neighbor_candidates(&docs, &budget, None).unwrap();

        assert!(
            candidates.is_empty(),
            "cross-project same-name docs should not auto-link without review-only override: {candidates:#?}"
        );
    }

    #[test]
    fn sword_spirit_uses_reranker_fallback_only_when_llm_decisions_are_unparseable() {
        assert!(!should_use_reranker_fallback("called"));
        assert!(should_use_reranker_fallback("called_unparseable_fallback"));
        assert!(should_use_reranker_fallback("not_called_no_candidates"));
    }

    #[test]
    fn sword_spirit_llm_parser_accepts_thinking_plus_text_blocks() {
        let body = r#"{
            "content": [
                {"type": "thinking", "thinking": "internal reasoning must not block the final text"},
                {"type": "text", "text": "[{\"candidate_id\":\"c1\",\"keep\":true,\"relation\":\"supports\",\"confidence\":0.9,\"rationale\":\"ok\"}]"}
            ]
        }"#;

        let text = extract_anthropic_text(body).unwrap();
        let decisions = parse_decisions(&text).unwrap();

        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].candidate_id, "c1");
        assert!(decisions[0].keep);
        assert_eq!(decisions[0].relation.as_deref(), Some("supports"));
    }

    #[test]
    fn sword_spirit_llm_parser_accepts_single_decision_object() {
        let decisions = parse_decisions(
            r#"{"candidate_id":"c1","keep":true,"relation":"supports","confidence":0.91,"rationale":"ok"}"#,
        )
        .unwrap();

        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].candidate_id, "c1");
        assert!(decisions[0].keep);
    }
}
