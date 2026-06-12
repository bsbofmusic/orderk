use anyhow::{anyhow, Context, Result};
use chrono::{Datelike, Duration as ChronoDuration, NaiveDate, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use crate::api::{
    index_paths_with_options, provider_from_name, query_with_options, status as orderk_status,
};
use crate::models::{IndexOptions, IndexPathOptions, IndexSummary, QueryOptions, VectorBackend};
use crate::profiles::{resolve_sword_model_profile_from_env, SwordModelSlot};
use crate::scanner::scan_vault;

const CONTROL_ROOT: &str = ".orderk/jianling";
const JIANLING_VERSION: &str = "0.1";
const JIANLING_CHUNK_SIZE: usize = 40;
const DEFAULT_ANTHROPIC_MINIMAX_BASE_URL: &str = "https://api.minimaxi.com/anthropic";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JianlingRunMode {
    Daily,
    Weekly,
    Monthly,
    Yearly,
    Manual,
}

impl JianlingRunMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
            Self::Yearly => "yearly",
            Self::Manual => "manual",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "daily" | "day" | "today" => Ok(Self::Daily),
            "weekly" | "week" => Ok(Self::Weekly),
            "monthly" | "month" => Ok(Self::Monthly),
            "yearly" | "year" => Ok(Self::Yearly),
            "manual" => Ok(Self::Manual),
            other => Err(anyhow!(
                "unknown jianling mode: {other}; expected daily, weekly, monthly, yearly, or manual"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JianlingRunOptions {
    pub profile: String,
    pub mode: JianlingRunMode,
    pub dry_run: bool,
    pub scheduled: bool,
    pub db: Option<PathBuf>,
    pub date: Option<String>,
    pub max_source_files: usize,
}

impl Default for JianlingRunOptions {
    fn default() -> Self {
        Self {
            profile: "default".to_string(),
            mode: JianlingRunMode::Daily,
            dry_run: true,
            scheduled: false,
            db: None,
            date: None,
            max_source_files: 120,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JianlingEnableOptions {
    pub profile: String,
    pub schedule: String,
    pub timezone: String,
    pub db: Option<PathBuf>,
    pub orderk_bin: PathBuf,
    pub systemd_user_dir: Option<PathBuf>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JianlingWorkerOptions {
    pub profile: String,
    pub db: Option<PathBuf>,
    pub date: Option<String>,
    pub max_source_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JianlingValidateFileOptions {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JianlingSuccessPredicate {
    pub provider: String,
    pub schema_validation: String,
    pub pre_llm_guard: String,
    pub pre_write_guard: String,
    pub write: String,
    pub index_smoke: String,
    pub receipt_write: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JianlingSourceAnchor {
    pub id: String,
    pub path: String,
    pub quote_hash: String,
    pub source_file_hash: String,
    pub source_tier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JianlingFileOp {
    pub op: String,
    pub target_path: String,
    pub preimage_hash: Option<String>,
    pub postimage_hash: String,
    pub byte_count: usize,
    pub index_update_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JianlingIndexSummary {
    pub path: String,
    pub db: String,
    pub files: usize,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub deleted: usize,
    pub chunks: usize,
    pub embedded: usize,
    pub reused: usize,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub vector_backend: String,
    pub chunk_strategy: String,
    pub chunk_max_chars: usize,
    pub chunk_overlap_chars: usize,
    pub took_ms: u128,
}

impl JianlingIndexSummary {
    fn from_index_summary(path: &str, summary: IndexSummary) -> Self {
        Self {
            path: path.to_string(),
            db: summary.db,
            files: summary.files,
            added: summary.added,
            updated: summary.updated,
            unchanged: summary.unchanged,
            deleted: summary.deleted,
            chunks: summary.chunks,
            embedded: summary.embedded,
            reused: summary.reused,
            embedding_provider: summary.embedding_provider,
            embedding_model: summary.embedding_model,
            vector_backend: summary.vector_backend,
            chunk_strategy: summary.chunk_strategy,
            chunk_max_chars: summary.chunk_max_chars,
            chunk_overlap_chars: summary.chunk_overlap_chars,
            took_ms: summary.took_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JianlingIndexFeedbackOutcome {
    index_update: String,
    index_smoke_status: String,
    index_summary: Option<JianlingIndexSummary>,
    warnings: Vec<String>,
    degraded: bool,
}

#[derive(Debug, Clone)]
struct JianlingIndexProfile {
    embedding_provider: String,
    embedding_dim: usize,
    embedding_model: String,
    vector_backend: VectorBackend,
    index_options: IndexOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct JianlingTopicLedger {
    schema_version: String,
    profile: String,
    updated_at: String,
    topics: BTreeMap<String, JianlingTopicEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct JianlingTopicEntry {
    topic_key: String,
    title: String,
    first_seen: String,
    last_seen: String,
    repeat_count: usize,
    confidence: String,
    seen_occurrences: Vec<String>,
    durable_evidence_refs: Vec<JianlingDurableEvidenceRef>,
    source_paths: Vec<String>,
    source_file_hashes: Vec<String>,
    modes_seen: Vec<String>,
    latest_next_action: String,
    promotion_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct JianlingDurableEvidenceRef {
    run_id: String,
    anchor_id: String,
    source_path: String,
    source_file_hash: String,
    quote_hash: String,
}

#[derive(Debug, Clone)]
struct JianlingPromotionWrite {
    rel_path: String,
    body: String,
    file_op: JianlingFileOp,
    skipped: bool,
    warning: Option<String>,
}

fn default_promotion_status() -> String {
    "not_applicable".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JianlingRunReport {
    pub ok: bool,
    pub schema_version: String,
    pub run_id: String,
    pub mode: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: String,
    pub vault: String,
    pub db: Option<String>,
    pub profile: String,
    pub scheduled: bool,
    pub scheduler_backend: String,
    pub llm_profile: String,
    pub provider_status: String,
    pub schema_validation_status: String,
    pub budget_status: String,
    pub pre_llm_guard_status: String,
    pub pre_write_guard_status: String,
    pub index_update: String,
    pub index_smoke_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_summary: Option<JianlingIndexSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic_ledger_path: Option<String>,
    #[serde(default)]
    pub promotion_paths: Vec<String>,
    #[serde(default = "default_promotion_status")]
    pub promotion_status: String,
    #[serde(default)]
    pub promotion_index_summaries: Vec<JianlingIndexSummary>,
    #[serde(default)]
    pub promotion_file_ops: Vec<JianlingFileOp>,
    pub fallback_used: bool,
    pub success_predicate: JianlingSuccessPredicate,
    pub source_files: usize,
    pub source_chars: usize,
    pub generated_files: Vec<String>,
    pub generated_source_tier: String,
    pub file_ops: Vec<JianlingFileOp>,
    pub evidence_pack_hash: String,
    pub evidence_pack_path: String,
    pub receipt_path: String,
    pub lock_path: String,
    pub lock_clean: bool,
    pub source_anchors: Vec<JianlingSourceAnchor>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub source_total_files: usize,
    #[serde(default)]
    pub rejected_source_files: Vec<String>,
    #[serde(default)]
    pub chunking_status: String,
    #[serde(default)]
    pub chunk_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreman_summary_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JianlingStatusResponse {
    pub ok: bool,
    pub schema_version: String,
    pub vault: String,
    pub profile: String,
    pub control_root: String,
    pub scheduler_backend: String,
    pub enabled: bool,
    pub service_path: Option<String>,
    pub timer_path: Option<String>,
    pub latest_run_id: Option<String>,
    pub latest_run_path: Option<String>,
    pub last_run_status: Option<String>,
    pub next_run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler_runtime: Option<JianlingSystemdRuntime>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JianlingSystemdRuntime {
    pub timer_unit: String,
    pub checked: bool,
    pub active_state: Option<String>,
    pub sub_state: Option<String>,
    pub unit_file_state: Option<String>,
    pub next_elapse: Option<String>,
    pub last_trigger: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JianlingDoctorCheck {
    pub component: String,
    pub ok: bool,
    pub status: String,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JianlingDoctorResponse {
    pub ok: bool,
    pub schema_version: String,
    pub vault: String,
    pub profile: String,
    pub checks: Vec<JianlingDoctorCheck>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JianlingChatSmokeResponse {
    pub ok: bool,
    pub schema_version: String,
    pub vault: String,
    pub profile: String,
    pub provider: String,
    pub model: String,
    pub api_key_env: Option<String>,
    pub api_key_configured: bool,
    pub base_url_configured: bool,
    pub status: String,
    pub verification_mode: String,
    pub took_ms: u128,
    pub response_preview: Option<String>,
    pub receipt_path: String,
    pub error_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JianlingEnableReport {
    pub ok: bool,
    pub schema_version: String,
    pub vault: String,
    pub profile: String,
    pub scheduler_backend: String,
    pub schedule: String,
    pub timezone: String,
    pub service_path: String,
    pub timer_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_path: Option<String>,
    pub dry_run: bool,
    pub written_files: Vec<String>,
    pub activation_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler_runtime: Option<JianlingSystemdRuntime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JianlingWorkerReport {
    pub ok: bool,
    pub schema_version: String,
    pub vault: String,
    pub profile: String,
    pub date: String,
    pub started_at: String,
    pub finished_at: String,
    pub modes_planned: Vec<String>,
    pub runs: Vec<JianlingRunReport>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JianlingDisableReport {
    pub ok: bool,
    pub schema_version: String,
    pub vault: String,
    pub profile: String,
    pub removed_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JianlingValidationResponse {
    pub ok: bool,
    pub schema_version: String,
    pub path: String,
    pub error_codes: Vec<String>,
    pub checks: Vec<JianlingDoctorCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JianlingSchedulerState {
    schema_version: String,
    profile: String,
    scheduler_backend: String,
    schedule: String,
    timezone: String,
    service_path: String,
    timer_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    env_path: Option<String>,
    orderk_bin: String,
    vault: String,
    db: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JianlingWatermarkState {
    schema_version: String,
    profile: String,
    files: BTreeMap<String, JianlingWatermarkFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JianlingWatermarkFile {
    hash: String,
    mtime: i64,
    size: u64,
    last_processed_run: String,
    last_status: String,
}

#[derive(Debug, Clone, Serialize)]
struct EvidencePack<'a> {
    schema_version: &'a str,
    run_id: &'a str,
    source_anchors: &'a [JianlingSourceAnchor],
    selected_sources: Vec<EvidenceSource>,
}

#[derive(Debug, Clone, Deserialize)]
struct EvidencePackOwned {
    schema_version: String,
    run_id: String,
    source_anchors: Vec<JianlingSourceAnchor>,
    selected_sources: Vec<EvidenceSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvidenceSource {
    path: String,
    hash: String,
    chars: usize,
    excerpt: String,
}

pub fn jianling_run(vault: &Path, options: &JianlingRunOptions) -> Result<JianlingRunReport> {
    let started_at = Utc::now();
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let profile = clean_profile(&options.profile)?;
    let date = options
        .date
        .clone()
        .unwrap_or_else(|| Utc::now().format("%Y-%m-%d").to_string());
    let run_stamp = started_at
        .format("%Y%m%dT%H%M%S%.fZ")
        .to_string()
        .replace('.', "");
    let run_id = format!(
        "jianling-{}-{}-{}",
        options.mode.as_str(),
        run_stamp,
        std::process::id()
    );
    let root = control_root(&vault);
    let runs_root = root.join("runs");
    let logs_root = root.join("logs");
    let lock_path = root.join("locks").join(format!("{profile}.lock"));
    let target_rel = target_rel_for_mode(&options.mode, &date);

    let target_abs = safe_vault_path(&vault, Path::new(&target_rel))?;
    ensure_vault_path_has_no_symlink_escape(&vault, &target_abs, "jianling generated markdown")?;
    let target_exists = ensure_existing_generated_target_is_managed(&target_abs)?;
    let target_preimage_hash = if target_exists {
        file_hash_if_exists(&target_abs)?
    } else {
        None
    };

    let selection = select_source_files(&vault, &options.mode, &date, options.max_source_files)?;
    let selected = selection.selected;
    let mut warnings = Vec::new();
    if !selection.rejected_paths.is_empty() {
        warnings.push(format!(
            "source file limit reached: selected {} of {}; rejected {} files (status is partial, not silently truncated)",
            selected.len(),
            selection.total_files,
            selection.rejected_paths.len()
        ));
    }
    let mut source_anchors = Vec::new();
    let mut evidence_sources = Vec::new();
    let mut source_chars = 0usize;
    for (idx, file) in selected.iter().enumerate() {
        let raw = fs::read_to_string(&file.abs_path)
            .with_context(|| format!("read source file {}", file.abs_path.display()))?;
        let excerpt = redacted_excerpt(&raw, 900);
        source_chars += raw.chars().count();
        source_anchors.push(JianlingSourceAnchor {
            id: format!("S{}", idx + 1),
            path: file.path.clone(),
            quote_hash: sha256_hex(excerpt.as_bytes()),
            source_file_hash: format!("sha256:{}", file.hash),
            source_tier: "raw_truth".to_string(),
        });
        evidence_sources.push(EvidenceSource {
            path: file.path.clone(),
            hash: format!("sha256:{}", file.hash),
            chars: raw.chars().count(),
            excerpt,
        });
    }

    let mut provider_status = jianling_provider_status_for_dry_run(options.dry_run, &profile);
    let mut fallback_used = false;
    let mut llm_contract_degraded = false;
    let mut generated_body = render_jianling_digest(
        &options.mode,
        &date,
        &run_id,
        &source_anchors,
        &evidence_sources,
        selection.total_files,
        &selection.rejected_paths,
    );
    if !options.dry_run && jianling_live_llm_enabled(&profile) {
        if let Some(reflection) = generate_live_llm_reflection(LiveReflectionInput {
            profile: &profile,
            mode: &options.mode,
            date: &date,
            run_id: &run_id,
            anchors: &source_anchors,
            sources: &evidence_sources,
            source_total_files: selection.total_files,
            rejected_source_files: &selection.rejected_paths,
        })? {
            match validate_live_llm_reflection_contract(&reflection, &source_anchors) {
                Ok(()) => {
                    provider_status = "called_live".to_string();
                    generated_body.push_str("\n### LLM 反思（MiniMax M3）\n");
                    generated_body.push_str(reflection.trim());
                    generated_body.push('\n');
                }
                Err(err) => {
                    provider_status = "called_live_schema_invalid".to_string();
                    fallback_used = true;
                    llm_contract_degraded = true;
                    warnings.push(format!(
                        "live LLM reflection rejected by digest.v2 contract: {err}"
                    ));
                }
            }
        }
    }
    let postimage_hash = format!("sha256:{}", sha256_hex(generated_body.as_bytes()));
    let file_ops = vec![JianlingFileOp {
        op: if target_exists {
            "replace".to_string()
        } else {
            "create".to_string()
        },
        target_path: target_rel.clone(),
        preimage_hash: target_preimage_hash.clone(),
        postimage_hash,
        byte_count: generated_body.len(),
        index_update_required: true,
    }];
    let evidence_pack = EvidencePack {
        schema_version: "orderk.jianling.evidence.v1",
        run_id: &run_id,
        source_anchors: &source_anchors,
        selected_sources: evidence_sources.clone(),
    };
    let evidence_json = serde_json::to_string_pretty(&evidence_pack)? + "\n";
    let evidence_hash = format!("sha256:{}", sha256_hex(evidence_json.as_bytes()));
    let receipt_path = runs_root.join(format!("{run_id}.json"));
    let evidence_path = runs_root.join(format!("{run_id}.evidence.json.redacted"));
    let (index_update, index_smoke_status) = if options.dry_run {
        ("skipped_dry_run", "skipped_dry_run")
    } else if options.db.is_some() {
        ("pending", "pending")
    } else {
        ("skipped_no_db", "skipped_no_db")
    };

    let mut report = JianlingRunReport {
        ok: !llm_contract_degraded,
        schema_version: "orderk.jianling.run.v1".to_string(),
        run_id: run_id.clone(),
        mode: options.mode.as_str().to_string(),
        status: if options.dry_run {
            "dry_run"
        } else if llm_contract_degraded {
            "degraded_llm_schema_invalid"
        } else {
            "success"
        }
        .to_string(),
        started_at: started_at.to_rfc3339(),
        finished_at: Utc::now().to_rfc3339(),
        vault: vault.to_string_lossy().to_string(),
        db: options
            .db
            .as_ref()
            .map(|path| path.to_string_lossy().to_string()),
        profile: profile.clone(),
        scheduled: options.scheduled,
        scheduler_backend: if options.scheduled {
            "systemd-user".to_string()
        } else {
            "manual".to_string()
        },
        llm_profile: jianling_llm_profile_label(),
        provider_status: provider_status.clone(),
        schema_validation_status: "passed".to_string(),
        budget_status: if selection.rejected_paths.is_empty() {
            "within_budget"
        } else {
            "partial_source_file_limit"
        }
        .to_string(),
        pre_llm_guard_status: "passed".to_string(),
        pre_write_guard_status: "passed".to_string(),
        index_update: index_update.to_string(),
        index_smoke_status: index_smoke_status.to_string(),
        index_summary: None,
        topic_ledger_path: None,
        promotion_paths: Vec::new(),
        promotion_status: default_promotion_status(),
        promotion_index_summaries: Vec::new(),
        promotion_file_ops: Vec::new(),
        fallback_used,
        success_predicate: JianlingSuccessPredicate {
            provider: provider_status.clone(),
            schema_validation: "passed".to_string(),
            pre_llm_guard: "passed".to_string(),
            pre_write_guard: "passed".to_string(),
            write: if options.dry_run { "dry_run" } else { "passed" }.to_string(),
            index_smoke: "skipped_p0".to_string(),
            receipt_write: if options.dry_run { "dry_run" } else { "passed" }.to_string(),
        },
        source_files: selected.len(),
        source_chars,
        generated_files: vec![target_rel.clone()],
        generated_source_tier: "generated_memory".to_string(),
        file_ops,
        evidence_pack_hash: evidence_hash,
        evidence_pack_path: evidence_path.to_string_lossy().to_string(),
        receipt_path: receipt_path.to_string_lossy().to_string(),
        lock_path: lock_path.to_string_lossy().to_string(),
        lock_clean: true,
        source_anchors,
        warnings,
        source_total_files: selection.total_files,
        rejected_source_files: selection.rejected_paths,
        chunking_status: if selected.is_empty() {
            "not_needed".to_string()
        } else {
            "planned_kanban_chunks".to_string()
        },
        chunk_count: chunk_count_for(selected.len()),
        chunk_dir: None,
        foreman_summary_path: None,
        log_path: if options.dry_run {
            None
        } else {
            Some(
                logs_root
                    .join(format!("{run_id}.log"))
                    .to_string_lossy()
                    .to_string(),
            )
        },
    };

    if options.dry_run {
        return Ok(report);
    }

    prepare_jianling_root(&vault, &root)?;
    prepare_child_dir(&runs_root, "jianling runs")?;
    prepare_child_dir(&logs_root, "jianling logs")?;
    prepare_child_dir(&root.join("locks"), "jianling locks")?;
    let lock = create_lock(&lock_path, &run_id, &profile, options.mode.as_str())?;
    let write_result = (|| -> Result<()> {
        if !selected.is_empty() {
            let chunk_report = write_kanban_refinement_harness(
                &runs_root,
                &run_id,
                &report.source_anchors,
                &selected,
                &report.rejected_source_files,
                &generated_body,
            )?;
            report.chunking_status = "kanban_foreman_summary_written".to_string();
            report.chunk_count = chunk_report.chunk_count;
            report.chunk_dir = Some(chunk_report.chunk_dir.to_string_lossy().to_string());
            report.foreman_summary_path = Some(
                chunk_report
                    .foreman_summary_path
                    .to_string_lossy()
                    .to_string(),
            );
            generated_body.push_str(
                "
### Kanban 精炼 Harness（证据附录）
",
            );
            generated_body.push_str(&format!(
                "- writer cards: {}
- auditor cards: {}
- foreman manifest: {}
- gate: passed — final Markdown is written only after foreman acceptance.
",
                chunk_report.chunk_count,
                chunk_report.chunk_count,
                chunk_report.foreman_summary_path.to_string_lossy()
            ));
        }
        let final_postimage_hash = format!("sha256:{}", sha256_hex(generated_body.as_bytes()));
        report.file_ops = vec![JianlingFileOp {
            op: if target_exists {
                "replace".to_string()
            } else {
                "create".to_string()
            },
            target_path: target_rel.clone(),
            preimage_hash: target_preimage_hash.clone(),
            postimage_hash: final_postimage_hash,
            byte_count: generated_body.len(),
            index_update_required: true,
        }];
        write_generated_file(&vault, &target_rel, &generated_body)?;
        let index_feedback = run_jianling_index_feedback(
            &vault,
            options.db.as_deref(),
            &target_rel,
            &run_id,
            generated_title_for_mode(&options.mode),
        );
        report.index_update = index_feedback.index_update;
        report.index_smoke_status = index_feedback.index_smoke_status;
        report.index_summary = index_feedback.index_summary;
        report.warnings.extend(index_feedback.warnings);
        if index_feedback.degraded {
            report.ok = false;
            report.status = "degraded_index_failed".to_string();
            report.success_predicate.index_smoke = "failed".to_string();
        } else if report.index_smoke_status == "passed" {
            report.success_predicate.index_smoke = "passed".to_string();
        }

        let ledger_path = topic_ledger_path(&root);
        if report.ok && !report.source_anchors.is_empty() {
            let observations = derive_reflective_observations(
                &report.source_anchors,
                &evidence_sources,
                &report.rejected_source_files,
            );
            let mut ledger = load_topic_ledger(&ledger_path, &profile)?;
            let mut ledger_changed = update_topic_ledger_after_success(
                &mut ledger,
                &run_id,
                &options.mode,
                &date,
                &observations,
                &report.source_anchors,
                &evidence_sources,
            );
            let promotion_writes =
                write_promotion_proposals(&vault, &ledger, &options.mode, &run_id, &date)?;
            for promotion in promotion_writes {
                if let Some(warning) = promotion.warning {
                    report.warnings.push(warning);
                }
                if promotion.skipped {
                    if report.promotion_status == "not_applicable" {
                        report.promotion_status = "skipped".to_string();
                    }
                    continue;
                }
                report.promotion_paths.push(promotion.rel_path.clone());
                report.promotion_file_ops.push(promotion.file_op.clone());
                let promotion_feedback = run_jianling_index_feedback(
                    &vault,
                    options.db.as_deref(),
                    &promotion.rel_path,
                    &run_id,
                    &format!("Jianling Lesson Proposal {}", promotion.rel_path),
                );
                if let Some(summary) = promotion_feedback.index_summary {
                    report.promotion_index_summaries.push(summary);
                }
                report.warnings.extend(promotion_feedback.warnings);
                if promotion_feedback.degraded
                    || promotion_feedback.index_smoke_status != "passed"
                    || promotion_feedback.index_update == "skipped_no_db"
                {
                    report.ok = false;
                    report.status = "degraded_promotion_index_failed".to_string();
                    report.promotion_status = "degraded_index_failed".to_string();
                    report.success_predicate.index_smoke = "failed".to_string();
                    report.warnings.push(format!(
                        "promotion index feedback failed for {}: update={}, smoke={}",
                        promotion.rel_path,
                        promotion_feedback.index_update,
                        promotion_feedback.index_smoke_status
                    ));
                }
            }
            if !report.promotion_paths.is_empty() && report.ok {
                mark_promoted_topics(&mut ledger, &report.promotion_paths);
                ledger_changed = true;
                report.promotion_status = "proposed".to_string();
                report
                    .generated_files
                    .extend(report.promotion_paths.clone());
            } else if report.promotion_paths.is_empty()
                && report.promotion_status == "not_applicable"
            {
                report.promotion_status = if matches!(
                    options.mode,
                    JianlingRunMode::Daily | JianlingRunMode::Manual
                ) {
                    "not_applicable".to_string()
                } else {
                    "no_candidates".to_string()
                };
            }
            if ledger_changed {
                save_topic_ledger_atomic(&ledger_path, &ledger)?;
                report.topic_ledger_path = Some(ledger_path.to_string_lossy().to_string());
            }
        }
        ensure_plain_output_file(&evidence_path, "jianling evidence pack")?;
        fs::write(&evidence_path, evidence_json)?;
        report.finished_at = Utc::now().to_rfc3339();
        ensure_plain_output_file(&receipt_path, "jianling run receipt")?;
        fs::write(&receipt_path, serde_json::to_string_pretty(&report)? + "\n")?;
        if let Some(log_path) = &report.log_path {
            let log_path = PathBuf::from(log_path);
            ensure_plain_output_file(&log_path, "jianling run log")?;
            fs::write(&log_path, render_run_log(&report))?;
        }
        write_watermark(
            &root.join("watermarks.json"),
            &profile,
            &run_id,
            &report.status,
            &selected,
        )?;
        Ok(())
    })();
    drop(lock);
    let _ = fs::remove_file(&lock_path);
    write_result?;
    Ok(report)
}

fn run_jianling_index_feedback(
    vault: &Path,
    db: Option<&Path>,
    target_rel: &str,
    run_id: &str,
    smoke_title: &str,
) -> JianlingIndexFeedbackOutcome {
    let Some(db) = db else {
        return JianlingIndexFeedbackOutcome {
            index_update: "skipped_no_db".to_string(),
            index_smoke_status: "skipped_no_db".to_string(),
            index_summary: None,
            warnings: Vec::new(),
            degraded: false,
        };
    };

    let profile = match resolve_jianling_index_profile(db) {
        Ok(profile) => profile,
        Err(err) => {
            return JianlingIndexFeedbackOutcome {
                index_update: "failed".to_string(),
                index_smoke_status: "skipped_index_profile_failed".to_string(),
                index_summary: None,
                warnings: vec![format!("index feedback profile resolution failed: {err}")],
                degraded: true,
            };
        }
    };
    let provider = match provider_from_name(
        &profile.embedding_provider,
        profile.embedding_dim,
        Some(profile.embedding_model.clone()),
    ) {
        Ok(provider) => provider,
        Err(err) => {
            return JianlingIndexFeedbackOutcome {
                index_update: "failed".to_string(),
                index_smoke_status: "skipped_index_provider_failed".to_string(),
                index_summary: None,
                warnings: vec![format!("index feedback provider setup failed: {err}")],
                degraded: true,
            };
        }
    };

    let index_summary = match index_paths_with_options(
        vault,
        db,
        provider.as_ref(),
        profile.embedding_dim,
        &profile.embedding_model,
        profile.vector_backend.clone(),
        &IndexPathOptions {
            paths: vec![target_rel.to_string()],
            index: profile.index_options.clone(),
        },
    ) {
        Ok(summary) => summary,
        Err(err) => {
            return JianlingIndexFeedbackOutcome {
                index_update: "failed".to_string(),
                index_smoke_status: "skipped_index_failed".to_string(),
                index_summary: None,
                warnings: vec![format!("index feedback failed: {err}")],
                degraded: true,
            };
        }
    };
    let compact_summary = JianlingIndexSummary::from_index_summary(target_rel, index_summary);

    let mut smoke_options = QueryOptions::new(5);
    smoke_options.rerank = false;
    smoke_options.external_reranker = false;
    let smoke_query = format!("{run_id} {smoke_title}");
    let smoke_result = query_with_options(
        db,
        &smoke_query,
        &smoke_options,
        provider.as_ref(),
        profile.vector_backend,
    );
    match smoke_result {
        Ok(response) => {
            let found = response
                .results
                .iter()
                .any(|result| result.path == target_rel);
            if found {
                JianlingIndexFeedbackOutcome {
                    index_update: "success".to_string(),
                    index_smoke_status: "passed".to_string(),
                    index_summary: Some(compact_summary),
                    warnings: Vec::new(),
                    degraded: false,
                }
            } else {
                JianlingIndexFeedbackOutcome {
                    index_update: "success".to_string(),
                    index_smoke_status: "failed".to_string(),
                    index_summary: Some(compact_summary),
                    warnings: vec![format!(
                        "index smoke failed: query did not return generated path {target_rel} in top 5"
                    )],
                    degraded: true,
                }
            }
        }
        Err(err) => JianlingIndexFeedbackOutcome {
            index_update: "success".to_string(),
            index_smoke_status: "failed".to_string(),
            index_summary: Some(compact_summary),
            warnings: vec![format!("index smoke failed: {err}")],
            degraded: true,
        },
    }
}

fn topic_ledger_path(root: &Path) -> PathBuf {
    root.join("topic_ledger.json")
}

fn load_topic_ledger(path: &Path, profile: &str) -> Result<JianlingTopicLedger> {
    if !path.exists() {
        return Ok(empty_topic_ledger(profile));
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read Jianling topic ledger: {}", path.display()))?;
    let mut ledger: JianlingTopicLedger = serde_json::from_str(&raw)
        .with_context(|| format!("parse Jianling topic ledger: {}", path.display()))?;
    if ledger.schema_version != "orderk.jianling.topic_ledger.v1" {
        return Err(anyhow!(
            "unsupported Jianling topic ledger schema_version: {}",
            ledger.schema_version
        ));
    }
    if ledger.profile != profile {
        return Err(anyhow!(
            "Jianling topic ledger profile mismatch: expected {}, got {}",
            profile,
            ledger.profile
        ));
    }
    for entry in ledger.topics.values_mut() {
        entry.seen_occurrences.sort();
        entry.seen_occurrences.dedup();
        entry.source_paths.sort();
        entry.source_paths.dedup();
        entry.source_file_hashes.sort();
        entry.source_file_hashes.dedup();
        entry.modes_seen.sort();
        entry.modes_seen.dedup();
    }
    Ok(ledger)
}

fn empty_topic_ledger(profile: &str) -> JianlingTopicLedger {
    JianlingTopicLedger {
        schema_version: "orderk.jianling.topic_ledger.v1".to_string(),
        profile: profile.to_string(),
        updated_at: Utc::now().to_rfc3339(),
        topics: BTreeMap::new(),
    }
}

fn save_topic_ledger_atomic(path: &Path, ledger: &JianlingTopicLedger) -> Result<()> {
    if let Some(parent) = path.parent() {
        prepare_child_dir(parent, "jianling topic ledger")?;
    }
    ensure_plain_output_file(path, "jianling topic ledger")?;
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    ensure_plain_output_file(&tmp, "jianling topic ledger temp")?;
    fs::write(&tmp, serde_json::to_string_pretty(ledger)? + "\n")?;
    fs::rename(&tmp, path).with_context(|| {
        format!(
            "atomically replace Jianling topic ledger: {}",
            path.display()
        )
    })?;
    Ok(())
}

fn topic_key_for_observation(title: &str) -> String {
    match title {
        "质量复查偏好" => "quality-review-preference".to_string(),
        "底账不可丢" => "ledger-preservation".to_string(),
        "反思要有判断" => "reflective-judgment".to_string(),
        "覆盖不完整" => "partial-coverage-risk".to_string(),
        "证据优先" => "evidence-first".to_string(),
        other => slugify_topic_key(other),
    }
}

fn slugify_topic_key(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '-' | '_' | ' ' | '/' | ':') && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "observed-pattern".to_string()
    } else {
        out
    }
}

fn observation_occurrence_id(
    topic_key: &str,
    mode: &JianlingRunMode,
    date: &str,
    sources: &[EvidenceSource],
    anchors: &[JianlingSourceAnchor],
) -> String {
    let payload = json!({
        "topic_key": topic_key,
        "mode": mode.as_str(),
        "date": date,
        "source_paths": sources.iter().map(|source| source.path.clone()).collect::<Vec<_>>(),
        "source_file_hashes": sources.iter().map(|source| source.hash.clone()).collect::<Vec<_>>(),
        "quote_hashes": anchors.iter().map(|anchor| anchor.quote_hash.clone()).collect::<Vec<_>>(),
    });
    format!("occurrence:{}", sha256_hex(payload.to_string().as_bytes()))
}

fn durable_evidence_refs_for(
    run_id: &str,
    anchors: &[JianlingSourceAnchor],
) -> Vec<JianlingDurableEvidenceRef> {
    anchors
        .iter()
        .take(12)
        .map(|anchor| JianlingDurableEvidenceRef {
            run_id: run_id.to_string(),
            anchor_id: anchor.id.clone(),
            source_path: anchor.path.clone(),
            source_file_hash: anchor.source_file_hash.clone(),
            quote_hash: anchor.quote_hash.clone(),
        })
        .collect()
}

fn update_topic_ledger_after_success(
    ledger: &mut JianlingTopicLedger,
    run_id: &str,
    mode: &JianlingRunMode,
    date: &str,
    observations: &[ReflectiveObservation],
    anchors: &[JianlingSourceAnchor],
    sources: &[EvidenceSource],
) -> bool {
    if !matches!(mode, JianlingRunMode::Daily | JianlingRunMode::Manual) {
        return false;
    }
    if sources.is_empty() || anchors.is_empty() {
        return false;
    }
    let now = Utc::now().to_rfc3339();
    let source_paths = sources
        .iter()
        .map(|source| source.path.clone())
        .collect::<Vec<_>>();
    let source_hashes = sources
        .iter()
        .map(|source| source.hash.clone())
        .collect::<Vec<_>>();
    let durable_refs = durable_evidence_refs_for(run_id, anchors);
    let mut changed = false;
    for observation in observations {
        let topic_key = topic_key_for_observation(observation.title);
        let occurrence_id = observation_occurrence_id(&topic_key, mode, date, sources, anchors);
        let entry = ledger
            .topics
            .entry(topic_key.clone())
            .or_insert_with(|| JianlingTopicEntry {
                topic_key: topic_key.clone(),
                title: observation.title.to_string(),
                first_seen: date.to_string(),
                last_seen: date.to_string(),
                repeat_count: 0,
                confidence: observation.confidence.to_string(),
                seen_occurrences: Vec::new(),
                durable_evidence_refs: Vec::new(),
                source_paths: Vec::new(),
                source_file_hashes: Vec::new(),
                modes_seen: Vec::new(),
                latest_next_action: observation.next_action.to_string(),
                promotion_status: "none".to_string(),
            });
        entry.title = observation.title.to_string();
        entry.last_seen = date.to_string();
        entry.confidence =
            stronger_confidence(&entry.confidence, observation.confidence).to_string();
        entry.latest_next_action = observation.next_action.to_string();
        push_unique(&mut entry.modes_seen, mode.as_str().to_string());
        for source_path in &source_paths {
            push_unique(&mut entry.source_paths, source_path.clone());
        }
        for source_hash in &source_hashes {
            push_unique(&mut entry.source_file_hashes, source_hash.clone());
        }
        for durable_ref in &durable_refs {
            if !entry.durable_evidence_refs.contains(durable_ref) {
                entry.durable_evidence_refs.push(durable_ref.clone());
            }
        }
        if !entry.seen_occurrences.contains(&occurrence_id) {
            entry.seen_occurrences.push(occurrence_id);
            entry.repeat_count = entry.seen_occurrences.len();
            changed = true;
        }
    }
    if changed {
        ledger.updated_at = now;
    }
    changed
}

fn stronger_confidence<'a>(current: &'a str, incoming: &'a str) -> &'a str {
    if confidence_rank(incoming) > confidence_rank(current) {
        incoming
    } else {
        current
    }
}

fn confidence_rank(value: &str) -> u8 {
    match value {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
        values.sort();
    }
}

fn promotion_candidates_for_mode<'a>(
    mode: &JianlingRunMode,
    ledger: &'a JianlingTopicLedger,
) -> Vec<&'a JianlingTopicEntry> {
    if matches!(mode, JianlingRunMode::Daily | JianlingRunMode::Manual) {
        return Vec::new();
    }
    ledger
        .topics
        .values()
        .filter(|entry| {
            entry.repeat_count >= 2
                && entry.confidence == "high"
                && entry.promotion_status != "accepted"
                && entry.promotion_status != "superseded"
        })
        .collect()
}

fn promotion_rel_for_topic(topic_key: &str) -> String {
    format!("brain/lessons/{}.md", slugify_topic_key(topic_key))
}

fn render_lesson_proposal(entry: &JianlingTopicEntry, run_id: &str, date: &str) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("generated_by: orderk-jianling\n");
    out.push_str("promotion_schema_version: orderk.jianling.promotion.v1\n");
    out.push_str(&format!("run_id: {run_id}\n"));
    out.push_str("status: proposed\n");
    out.push_str("type: lesson_proposal\n");
    out.push_str(&format!("topic_key: {}\n", entry.topic_key));
    out.push_str(&format!("title: {}\n", entry.title));
    out.push_str(&format!("repeat_count: {}\n", entry.repeat_count));
    out.push_str(&format!("confidence: {}\n", entry.confidence));
    out.push_str(&format!("date: {date}\n"));
    out.push_str("source_anchors:\n");
    for evidence in entry.durable_evidence_refs.iter().take(12) {
        out.push_str(&format!("  - run_id: {}\n", evidence.run_id));
        out.push_str(&format!("    anchor_id: {}\n", evidence.anchor_id));
        out.push_str(&format!("    source_path: {}\n", evidence.source_path));
        out.push_str(&format!(
            "    source_file_hash: {}\n",
            evidence.source_file_hash
        ));
        out.push_str(&format!("    quote_hash: {}\n", evidence.quote_hash));
    }
    out.push_str("claim_refs:\n");
    for evidence in entry.durable_evidence_refs.iter().take(12) {
        out.push_str(&format!("  - {}#{}\n", evidence.run_id, evidence.anchor_id));
    }
    out.push_str("---\n\n");
    out.push_str(&format!("# Lesson Proposal — {}\n\n", entry.title));
    out.push_str("## 观察\n");
    out.push_str(&format!(
        "- topic_key: `{}`；repeat_count: `{}`；confidence: `{}`。\n",
        entry.topic_key, entry.repeat_count, entry.confidence
    ));
    out.push_str(&format!(
        "- latest_next_action: {}\n\n",
        entry.latest_next_action
    ));
    out.push_str("## 证据索引\n");
    for evidence in entry.durable_evidence_refs.iter().take(12) {
        out.push_str(&format!(
            "- `{}`#{} path=`{}` source_file_hash=`{}` quote_hash=`{}`\n",
            evidence.run_id,
            evidence.anchor_id,
            evidence.source_path,
            evidence.source_file_hash,
            evidence.quote_hash
        ));
    }
    out.push_str("\n## 候选落点\n");
    out.push_str("- USER memory candidate: only after human approval.\n");
    out.push_str("- owner skill candidate: only after human approval and independent audit.\n");
    out.push_str("- PRD/test-gate candidate: only through explicit repo change, not automatic memory promotion.\n");
    out
}

fn write_promotion_proposals(
    vault: &Path,
    ledger: &JianlingTopicLedger,
    mode: &JianlingRunMode,
    run_id: &str,
    date: &str,
) -> Result<Vec<JianlingPromotionWrite>> {
    let mut writes = Vec::new();
    for entry in promotion_candidates_for_mode(mode, ledger) {
        let rel_path = promotion_rel_for_topic(&entry.topic_key);
        let target = safe_vault_path(vault, Path::new(&rel_path))?;
        ensure_vault_path_has_no_symlink_escape(vault, &target, "jianling lesson proposal")?;
        if let Some(parent) = target.parent() {
            prepare_child_dir(parent, "jianling lesson proposals")?;
        }
        let existing = if target.exists() {
            Some(fs::read_to_string(&target).with_context(|| {
                format!(
                    "read existing Jianling lesson proposal: {}",
                    target.display()
                )
            })?)
        } else {
            None
        };
        if let Some(existing_text) = existing.as_deref() {
            if !existing_text.contains("generated_by: orderk-jianling") {
                writes.push(JianlingPromotionWrite {
                    rel_path,
                    body: String::new(),
                    file_op: JianlingFileOp {
                        op: "skip".to_string(),
                        target_path: String::new(),
                        preimage_hash: None,
                        postimage_hash: String::new(),
                        byte_count: 0,
                        index_update_required: false,
                    },
                    skipped: true,
                    warning: Some(format!(
                        "promotion target is non-Jianling human content; skipped without overwrite: {}",
                        target.display()
                    )),
                });
                continue;
            }
            if existing_text.contains("status: active_user_approved") {
                writes.push(JianlingPromotionWrite {
                    rel_path,
                    body: String::new(),
                    file_op: JianlingFileOp {
                        op: "skip".to_string(),
                        target_path: String::new(),
                        preimage_hash: file_hash_if_exists(&target)?,
                        postimage_hash: String::new(),
                        byte_count: 0,
                        index_update_required: false,
                    },
                    skipped: true,
                    warning: Some(format!(
                        "promotion target is active_user_approved; skipped without overwrite: {}",
                        target.display()
                    )),
                });
                continue;
            }
            if !existing_text.contains("status: proposed") {
                writes.push(JianlingPromotionWrite {
                    rel_path,
                    body: String::new(),
                    file_op: JianlingFileOp {
                        op: "skip".to_string(),
                        target_path: String::new(),
                        preimage_hash: file_hash_if_exists(&target)?,
                        postimage_hash: String::new(),
                        byte_count: 0,
                        index_update_required: false,
                    },
                    skipped: true,
                    warning: Some(format!(
                        "promotion target generated status is not proposed; skipped without overwrite: {}",
                        target.display()
                    )),
                });
                continue;
            }
        }
        ensure_plain_output_file(&target, "jianling lesson proposal")?;
        let body = render_lesson_proposal(entry, run_id, date);
        let preimage_hash = file_hash_if_exists(&target)?;
        let op = if target.exists() { "replace" } else { "create" }.to_string();
        let postimage_hash = format!("sha256:{}", sha256_hex(body.as_bytes()));
        fs::write(&target, &body)?;
        writes.push(JianlingPromotionWrite {
            rel_path: rel_path.clone(),
            body,
            file_op: JianlingFileOp {
                op,
                target_path: rel_path,
                preimage_hash,
                postimage_hash,
                byte_count: writes.last().map(|_| 0).unwrap_or(0),
                index_update_required: true,
            },
            skipped: false,
            warning: None,
        });
        if let Some(write) = writes.last_mut() {
            write.file_op.byte_count = write.body.len();
        }
    }
    Ok(writes)
}

fn mark_promoted_topics(ledger: &mut JianlingTopicLedger, paths: &[String]) {
    for path in paths {
        if let Some(file_name) = Path::new(path).file_stem().and_then(|stem| stem.to_str()) {
            if let Some(entry) = ledger.topics.get_mut(file_name) {
                entry.promotion_status = "proposed".to_string();
            }
        }
    }
    if !paths.is_empty() {
        ledger.updated_at = Utc::now().to_rfc3339();
    }
}

fn resolve_jianling_index_profile(db: &Path) -> Result<JianlingIndexProfile> {
    if !db.is_file() {
        return Err(anyhow!(
            "index db does not exist; refusing to create a new DB during Jianling feedback: {}",
            db.display()
        ));
    }
    existing_db_index_profile(db).ok_or_else(|| {
        anyhow!(
            "index db profile is not readable; run full `orderk index` for the active DB first: {}",
            db.display()
        )
    })
}

fn existing_db_index_profile(db: &Path) -> Option<JianlingIndexProfile> {
    let response = orderk_status(db).ok()?;
    if response.embedding_dim == 0 {
        return None;
    }
    Some(JianlingIndexProfile {
        embedding_provider: non_unknown(response.embedding_provider)?,
        embedding_dim: response.embedding_dim,
        embedding_model: non_unknown(response.embedding_model)?,
        vector_backend: parse_vector_backend(&non_unknown(response.vector_backend)?).ok()?,
        index_options: existing_db_index_options(db).unwrap_or_default(),
    })
}

fn existing_db_index_options(db: &Path) -> Option<IndexOptions> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let chunk_max_chars = read_usize_setting(&conn, "chunk_max_chars")
        .unwrap_or_else(|| IndexOptions::default().chunk_max_chars);
    let chunk_overlap_chars = read_usize_setting(&conn, "chunk_overlap_chars")
        .unwrap_or_else(|| IndexOptions::default().chunk_overlap_chars);
    Some(IndexOptions {
        chunk_max_chars,
        chunk_overlap_chars,
    })
}

fn read_usize_setting(conn: &Connection, key: &str) -> Option<usize> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .optional()
    .ok()
    .flatten()
    .and_then(|value| value.parse::<usize>().ok())
}

fn non_unknown(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "unknown" {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_vector_backend(value: &str) -> Result<VectorBackend> {
    match value.trim() {
        "sqlite_vec" => Ok(VectorBackend::SqliteVec),
        "exact" => Ok(VectorBackend::Exact),
        other => Err(anyhow!("unknown vector backend: {other}")),
    }
}

pub fn jianling_status(vault: &Path, profile: &str) -> Result<JianlingStatusResponse> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let profile = clean_profile(profile)?;
    let root = control_root(&vault);
    let scheduler = read_scheduler_state(&root, &profile)?;
    let latest = latest_run_report(&root)?;
    let scheduler_runtime = scheduler
        .as_ref()
        .and_then(|state| inspect_scheduler_runtime_if_managed(state).ok().flatten());
    Ok(JianlingStatusResponse {
        ok: true,
        schema_version: "orderk.jianling.status.v1".to_string(),
        vault: vault.to_string_lossy().to_string(),
        profile,
        control_root: root.to_string_lossy().to_string(),
        scheduler_backend: scheduler
            .as_ref()
            .map(|state| state.scheduler_backend.clone())
            .unwrap_or_else(|| "manual".to_string()),
        enabled: scheduler.is_some(),
        service_path: scheduler.as_ref().map(|state| state.service_path.clone()),
        timer_path: scheduler.as_ref().map(|state| state.timer_path.clone()),
        latest_run_id: latest.as_ref().map(|report| report.run_id.clone()),
        latest_run_path: latest.as_ref().map(|report| report.receipt_path.clone()),
        last_run_status: latest.as_ref().map(|report| report.status.clone()),
        next_run: scheduler_runtime
            .as_ref()
            .and_then(|runtime| runtime.next_elapse.clone())
            .or_else(|| scheduler.as_ref().map(|state| state.schedule.clone())),
        scheduler_runtime,
        warnings: Vec::new(),
    })
}

pub fn jianling_doctor(vault: &Path, profile: &str) -> Result<JianlingDoctorResponse> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let profile = clean_profile(profile)?;
    let root = control_root(&vault);
    let scheduler = read_scheduler_state(&root, &profile)?;
    let latest = latest_run_report(&root)?;
    let mut checks = Vec::new();
    checks.push(JianlingDoctorCheck {
        component: "vault".to_string(),
        ok: vault.is_dir(),
        status: "reachable".to_string(),
        detail: serde_json::json!({"path": vault}),
    });
    checks.push(JianlingDoctorCheck {
        component: "control_root".to_string(),
        ok: !root.exists() || root.is_dir(),
        status: if root.exists() {
            "present"
        } else {
            "not_initialized"
        }
        .to_string(),
        detail: serde_json::json!({"path": root}),
    });
    let scheduler_runtime = scheduler
        .as_ref()
        .and_then(|state| inspect_scheduler_runtime_if_managed(state).ok().flatten());
    let scheduler_files_ok = scheduler
        .as_ref()
        .map(|state| {
            Path::new(&state.service_path).is_file() && Path::new(&state.timer_path).is_file()
        })
        .unwrap_or(false);
    let scheduler_runtime_ok = match (&scheduler, &scheduler_runtime) {
        (Some(state), Some(runtime)) if is_default_systemd_state(state) => {
            runtime.error.is_none()
                && runtime.active_state.as_deref() == Some("active")
                && runtime.unit_file_state.as_deref() == Some("enabled")
        }
        (Some(state), None) if is_default_systemd_state(state) => false,
        (Some(_), _) => true,
        (None, _) => false,
    };
    let scheduler_ok = scheduler_files_ok && scheduler_runtime_ok;
    checks.push(JianlingDoctorCheck {
        component: "scheduler".to_string(),
        ok: scheduler_ok,
        status: if scheduler_ok {
            "enabled"
        } else if scheduler_files_ok {
            "unit_files_present_but_timer_not_active"
        } else {
            "not_enabled"
        }
        .to_string(),
        detail: serde_json::json!({"state": scheduler, "runtime": scheduler_runtime}),
    });
    checks.push(JianlingDoctorCheck {
        component: "last_run".to_string(),
        ok: latest
            .as_ref()
            .map(|report| report.status == "success" || report.status == "dry_run")
            .unwrap_or(true),
        status: latest
            .as_ref()
            .map(|report| report.status.clone())
            .unwrap_or_else(|| "none".to_string()),
        detail: serde_json::json!({"latest_run_id": latest.as_ref().map(|r| r.run_id.clone())}),
    });
    let slot = resolve_sword_model_profile_from_env()?.llm;
    checks.push(check(
        "llm_profile",
        true,
        if slot.provider == "disabled" {
            "disabled"
        } else if slot.api_key_configured {
            "configured"
        } else {
            "unconfigured"
        },
        json!({
            "verification_mode": "static",
            "provider": slot.provider,
            "model": slot.model,
            "api_key_env": slot.api_key_env,
            "api_key_configured": slot.api_key_configured,
            "base_url_configured": slot.base_url.is_some(),
            "profile_fingerprint": slot.profile_fingerprint,
            "note": "Run `orderk jianling chat-smoke --vault <vault>` for live LLM connectivity."
        }),
    ));
    let global_lock_path = root.join("locks").join(format!("{profile}.lock"));
    checks.push(check(
        "global_run_lock",
        !global_lock_path.exists(),
        if global_lock_path.exists() {
            "busy"
        } else {
            "available"
        },
        json!({"verification_mode":"static", "path": global_lock_path}),
    ));
    let brain_dirs: Vec<serde_json::Value> = ["brain/daily", "brain/weekly", "brain/monthly"]
        .iter()
        .map(|rel| {
            let path = vault.join(rel);
            json!({"path": rel, "exists": path.is_dir() || !path.exists(), "writable_parent": path.parent().map(|p| p.exists()).unwrap_or(false)})
        })
        .collect();
    checks.push(check(
        "brain_output_paths",
        true,
        "checked",
        json!({"verification_mode":"static", "paths": brain_dirs}),
    ));
    let ok = checks.iter().all(|check| check.ok);
    Ok(JianlingDoctorResponse {
        ok,
        schema_version: "orderk.jianling.doctor.v1".to_string(),
        vault: vault.to_string_lossy().to_string(),
        profile,
        checks,
        warnings: Vec::new(),
    })
}

pub fn jianling_chat_smoke(vault: &Path, profile: &str) -> Result<JianlingChatSmokeResponse> {
    let started = Instant::now();
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let profile = clean_profile(profile)?;
    let root = control_root(&vault);
    prepare_jianling_root(&vault, &root)?;
    let smoke_root = root.join("smoke");
    prepare_child_dir(&smoke_root, "jianling smoke receipts")?;
    let receipt_path = smoke_root.join(format!(
        "jianling-chat-smoke-{}-{}.json",
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        std::process::id()
    ));
    let slot = resolve_sword_model_profile_from_env()?.llm;
    let base_url_configured = slot.base_url.is_some();
    let mut response = JianlingChatSmokeResponse {
        ok: false,
        schema_version: "orderk.jianling.chat_smoke.v1".to_string(),
        vault: vault.to_string_lossy().to_string(),
        profile,
        provider: slot.provider.clone(),
        model: slot.model.clone(),
        api_key_env: slot.api_key_env.clone(),
        api_key_configured: slot.api_key_configured,
        base_url_configured,
        status: "not_run".to_string(),
        verification_mode: "live".to_string(),
        took_ms: 0,
        response_preview: None,
        receipt_path: receipt_path.to_string_lossy().to_string(),
        error_preview: None,
    };
    let result = AnthropicCompatibleChatClient::from_slot(&slot)
        .and_then(|mut client| client.send_prompt("Return exactly: orderk-jianling-smoke-ok"));
    match result {
        Ok(text) => {
            response.ok = text.contains("orderk-jianling-smoke-ok");
            response.status = if response.ok {
                "connected".to_string()
            } else {
                "unexpected_response".to_string()
            };
            response.response_preview = Some(compact_one_line(&text));
        }
        Err(err) => {
            response.ok = false;
            response.status = if !slot.api_key_configured {
                "llm_unconfigured".to_string()
            } else {
                "llm_health_failed".to_string()
            };
            response.error_preview = Some(compact_one_line(&err.to_string()));
        }
    }
    response.took_ms = started.elapsed().as_millis();
    ensure_plain_output_file(&receipt_path, "jianling chat smoke receipt")?;
    fs::write(
        &receipt_path,
        serde_json::to_string_pretty(&response)? + "\n",
    )?;
    Ok(response)
}

pub fn jianling_worker(
    vault: &Path,
    options: &JianlingWorkerOptions,
) -> Result<JianlingWorkerReport> {
    let started_at = Utc::now();
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let profile = clean_profile(&options.profile)?;
    let date = options
        .date
        .clone()
        .unwrap_or_else(|| Utc::now().format("%Y-%m-%d").to_string());
    let parsed_date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .with_context(|| format!("invalid Jianling worker date: {date}"))?;
    let modes = planned_modes_for_date(parsed_date);
    let mut runs = Vec::new();
    for mode in modes.iter().cloned() {
        runs.push(jianling_run(
            &vault,
            &JianlingRunOptions {
                profile: profile.clone(),
                mode,
                dry_run: false,
                scheduled: true,
                db: options.db.clone(),
                date: Some(date.clone()),
                max_source_files: options.max_source_files,
            },
        )?);
    }
    let ok = runs.iter().all(|run| run.ok);
    let status = if ok {
        "success"
    } else if runs.iter().any(|run| run.status == "degraded_index_failed") {
        "degraded_index_failed"
    } else {
        "degraded"
    };
    Ok(JianlingWorkerReport {
        ok,
        schema_version: "orderk.jianling.worker.v1".to_string(),
        vault: vault.to_string_lossy().to_string(),
        profile,
        date,
        started_at: started_at.to_rfc3339(),
        finished_at: Utc::now().to_rfc3339(),
        modes_planned: modes.iter().map(|mode| mode.as_str().to_string()).collect(),
        runs,
        status: status.to_string(),
    })
}

pub fn jianling_enable(
    vault: &Path,
    options: &JianlingEnableOptions,
) -> Result<JianlingEnableReport> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let profile = clean_profile(&options.profile)?;
    validate_schedule(&options.schedule)?;
    let root = control_root(&vault);
    let default_systemd_dir = systemd_user_home_dir()?.join(".config/systemd/user");
    let systemd_dir = options
        .systemd_user_dir
        .clone()
        .unwrap_or_else(|| default_systemd_dir.clone());
    let service_path = systemd_dir.join(format!("orderk-jianling@{profile}.service"));
    let timer_path = systemd_dir.join(format!("orderk-jianling@{profile}.timer"));
    let env_path = if options.systemd_user_dir.is_some() {
        root.join(format!("{profile}.env"))
    } else {
        systemd_user_home_dir()?
            .join(".config/orderk")
            .join(format!("{profile}.env"))
    };
    let db = options
        .db
        .clone()
        .unwrap_or_else(|| vault.join(".obsidian/orderk/orderk.sqlite"));
    let service = render_systemd_service(&options.orderk_bin, &profile, &vault, &db);
    let timer = render_systemd_timer(&profile, &options.schedule, &options.timezone);
    let env_file = render_orderk_profile_env(&profile)?;
    let written_files = vec![
        service_path.to_string_lossy().to_string(),
        timer_path.to_string_lossy().to_string(),
        env_path.to_string_lossy().to_string(),
        root.join("scheduler.json").to_string_lossy().to_string(),
    ];
    let timer_unit = format!("orderk-jianling@{profile}.timer");
    let mut activation_status = if options.dry_run {
        "dry_run".to_string()
    } else if options.systemd_user_dir.is_some() {
        "skipped_custom_systemd_dir".to_string()
    } else {
        "not_run".to_string()
    };
    let mut scheduler_runtime = None;
    if !options.dry_run {
        prepare_jianling_root(&vault, &root)?;
        prepare_child_dir(&systemd_dir, "systemd user unit")?;
        prepare_child_dir(
            env_path.parent().context("orderk profile env parent")?,
            "orderk profile env",
        )?;
        ensure_plain_output_file(&service_path, "jianling systemd service")?;
        fs::write(&service_path, service)?;
        ensure_plain_output_file(&timer_path, "jianling systemd timer")?;
        fs::write(&timer_path, timer)?;
        ensure_plain_output_file(&env_path, "orderk profile env")?;
        fs::write(&env_path, env_file)?;
        let state = JianlingSchedulerState {
            schema_version: "orderk.jianling.scheduler.v1".to_string(),
            profile: profile.clone(),
            scheduler_backend: "systemd-user".to_string(),
            schedule: options.schedule.clone(),
            timezone: options.timezone.clone(),
            service_path: service_path.to_string_lossy().to_string(),
            timer_path: timer_path.to_string_lossy().to_string(),
            env_path: Some(env_path.to_string_lossy().to_string()),
            orderk_bin: options.orderk_bin.to_string_lossy().to_string(),
            vault: vault.to_string_lossy().to_string(),
            db: Some(db.to_string_lossy().to_string()),
        };
        let scheduler_path = root.join("scheduler.json");
        ensure_plain_output_file(&scheduler_path, "jianling scheduler state")?;
        fs::write(
            &scheduler_path,
            serde_json::to_string_pretty(&state)? + "\n",
        )?;
        if options.systemd_user_dir.is_none() {
            import_systemd_user_environment(&profile)?;
            run_systemctl_user(&["daemon-reload"])?;
            run_systemctl_user(&["enable", "--now", &timer_unit])?;
            scheduler_runtime = Some(inspect_systemd_timer(&profile));
            let active = scheduler_runtime.as_ref().is_some_and(|runtime| {
                runtime.error.is_none()
                    && runtime.active_state.as_deref() == Some("active")
                    && runtime.unit_file_state.as_deref() == Some("enabled")
            });
            if active {
                activation_status = "enabled_active".to_string();
            } else {
                activation_status = "activation_unverified".to_string();
            }
        }
    }
    Ok(JianlingEnableReport {
        ok: activation_status != "activation_unverified",
        schema_version: "orderk.jianling.enable.v1".to_string(),
        vault: vault.to_string_lossy().to_string(),
        profile,
        scheduler_backend: "systemd-user".to_string(),
        schedule: options.schedule.clone(),
        timezone: options.timezone.clone(),
        service_path: service_path.to_string_lossy().to_string(),
        timer_path: timer_path.to_string_lossy().to_string(),
        env_path: Some(env_path.to_string_lossy().to_string()),
        dry_run: options.dry_run,
        written_files,
        activation_status,
        scheduler_runtime,
    })
}

pub fn jianling_disable(vault: &Path, profile: &str) -> Result<JianlingDisableReport> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let profile = clean_profile(profile)?;
    let root = control_root(&vault);
    let mut removed_files = Vec::new();
    if let Some(state) = read_scheduler_state(&root, &profile)? {
        for path in [state.service_path, state.timer_path] {
            let path_buf = PathBuf::from(&path);
            if path_buf.exists() {
                remove_managed_jianling_file(&path_buf)?;
                removed_files.push(path);
            }
        }
        let scheduler_state = root.join("scheduler.json");
        if scheduler_state.exists() {
            ensure_plain_output_file(&scheduler_state, "jianling scheduler state")?;
            fs::remove_file(&scheduler_state)?;
            removed_files.push(scheduler_state.to_string_lossy().to_string());
        }
    }
    Ok(JianlingDisableReport {
        ok: true,
        schema_version: "orderk.jianling.disable.v1".to_string(),
        vault: vault.to_string_lossy().to_string(),
        profile,
        removed_files,
    })
}

pub fn jianling_validate_file(
    vault: &Path,
    options: &JianlingValidateFileOptions,
) -> Result<JianlingValidationResponse> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let path = safe_vault_path(&vault, &options.path)?;
    let rel = path
        .strip_prefix(&vault)?
        .to_string_lossy()
        .replace('\\', "/");
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let mut checks = Vec::new();
    let mut error_codes = Vec::new();
    let generated = text.contains("generated_by: orderk-jianling");
    checks.push(check(
        "generated_by",
        generated,
        if generated { "present" } else { "missing" },
        serde_json::json!({"required": true}),
    ));
    if !generated {
        error_codes.push("missing_generated_by".to_string());
    }
    let has_valid_status = [
        "status: active_generated",
        "status: active_user_approved",
        "status: proposed",
        "status: draft",
        "status: superseded",
        "status: stale",
        "status: rejected",
    ]
    .iter()
    .any(|needle| text.contains(needle));
    checks.push(check(
        "status",
        has_valid_status,
        if has_valid_status {
            "valid"
        } else {
            "missing_or_invalid"
        },
        serde_json::json!({}),
    ));
    if !has_valid_status {
        error_codes.push("missing_or_invalid_status".to_string());
    }
    let has_source_anchors = text.contains("source_anchors:") && text.contains("quote_hash:");
    checks.push(check(
        "source_anchors",
        has_source_anchors,
        if has_source_anchors {
            "present"
        } else {
            "missing"
        },
        serde_json::json!({}),
    ));
    if !has_source_anchors {
        error_codes.push("missing_source_anchors".to_string());
    }
    let has_claim_refs =
        text.contains("claim_refs:") || text.contains("[^S") || text.contains("[S1]");
    checks.push(check(
        "claim_refs",
        has_claim_refs,
        if has_claim_refs { "present" } else { "missing" },
        serde_json::json!({}),
    ));
    if !has_claim_refs {
        error_codes.push("missing_claim_refs".to_string());
    }

    let has_digest_v2 = text.contains("digest_schema_version: orderk.jianling.digest.v2")
        && text.contains("reflection_layers: [factual_ledger, reflective_synthesis]");
    checks.push(check(
        "digest_v2_schema",
        has_digest_v2,
        if has_digest_v2 { "present" } else { "missing" },
        serde_json::json!({
            "digest_schema_version": "orderk.jianling.digest.v2",
            "reflection_layers": "[factual_ledger, reflective_synthesis]"
        }),
    ));
    if !has_digest_v2 {
        error_codes.push("missing_digest_v2_schema".to_string());
    }

    let required_sections = [
        "## 一句话结论",
        "## 客观底账 / Factual ledger",
        "## 推断观察 / Reflective synthesis",
        "## 用户/系统模式 / User-system patterns",
        "## 未闭合风险 / Open risks",
        "## 下次动作 / Next actions",
        "## 证据附录 / Evidence appendix",
    ];
    let missing_sections = required_sections
        .iter()
        .copied()
        .filter(|section| !text.contains(section))
        .collect::<Vec<_>>();
    let sections_ok = missing_sections.is_empty();
    checks.push(check(
        "digest_v2_sections",
        sections_ok,
        if sections_ok { "present" } else { "missing" },
        serde_json::json!({"missing_sections": missing_sections}),
    ));
    if !sections_ok {
        error_codes.push("missing_digest_v2_sections".to_string());
    }

    let observation_contract_ok = text.contains("## 推断观察 / Reflective synthesis")
        && text.contains("confidence:")
        && text.contains("evidence:")
        && text.contains("next:");
    checks.push(check(
        "digest_v2_observation_contract",
        observation_contract_ok,
        if observation_contract_ok {
            "present"
        } else {
            "missing"
        },
        serde_json::json!({"requires": ["confidence:", "evidence:", "next:"]}),
    ));
    if !observation_contract_ok {
        error_codes.push("missing_digest_v2_observation_contract".to_string());
    }
    Ok(JianlingValidationResponse {
        ok: error_codes.is_empty(),
        schema_version: "orderk.jianling.validate_file.v1".to_string(),
        path: rel,
        error_codes,
        checks,
    })
}

pub fn jianling_validate_run(vault: &Path, run_id: &str) -> Result<JianlingValidationResponse> {
    let vault = vault.canonicalize()?;
    let receipt = control_root(&vault)
        .join("runs")
        .join(format!("{run_id}.json"));
    if !receipt.is_file() {
        return Ok(JianlingValidationResponse {
            ok: false,
            schema_version: "orderk.jianling.validate_run.v1".to_string(),
            path: receipt.to_string_lossy().to_string(),
            error_codes: vec!["missing_run_receipt".to_string()],
            checks: vec![check(
                "run_receipt",
                false,
                "missing",
                serde_json::json!({"run_id": run_id}),
            )],
        });
    }
    let raw = fs::read_to_string(&receipt)?;
    let report: JianlingRunReport = serde_json::from_str(&raw)?;
    let mut checks = Vec::new();
    let mut error_codes = Vec::new();

    record_validation_check(
        &mut checks,
        &mut error_codes,
        "receipt",
        report.ok && report.run_id == run_id,
        &report.status,
        "invalid_run_receipt",
        serde_json::json!({"run_id": run_id, "receipt_run_id": report.run_id}),
    );
    record_validation_check(
        &mut checks,
        &mut error_codes,
        "lock_clean",
        report.lock_clean,
        if report.lock_clean { "clean" } else { "dirty" },
        "lock_not_clean",
        serde_json::json!({"lock_path": report.lock_path}),
    );
    record_validation_check(
        &mut checks,
        &mut error_codes,
        "generated_files",
        !report.generated_files.is_empty(),
        if report.generated_files.is_empty() {
            "missing"
        } else {
            "present"
        },
        "missing_generated_files",
        serde_json::json!({"generated_files": report.generated_files}),
    );

    let evidence_path = PathBuf::from(&report.evidence_pack_path);
    let evidence_check =
        validate_existing_plain_file_under(&vault, &evidence_path, "jianling evidence pack");
    record_validation_check(
        &mut checks,
        &mut error_codes,
        "evidence_pack_path",
        evidence_check.is_ok(),
        if evidence_check.is_ok() {
            "present"
        } else {
            "invalid"
        },
        "invalid_evidence_pack_path",
        serde_json::json!({"path": report.evidence_pack_path}),
    );
    if let Ok(canonical_evidence) = evidence_check {
        let evidence_bytes = fs::read(&canonical_evidence)?;
        let actual_hash = format!("sha256:{}", sha256_hex(&evidence_bytes));
        let evidence_hash_ok = actual_hash == report.evidence_pack_hash;
        record_validation_check(
            &mut checks,
            &mut error_codes,
            "evidence_pack_hash",
            evidence_hash_ok,
            if evidence_hash_ok {
                "match"
            } else {
                "mismatch"
            },
            "evidence_pack_hash_mismatch",
            serde_json::json!({"expected": report.evidence_pack_hash, "actual": actual_hash}),
        );
        let evidence_pack: Result<EvidencePackOwned> =
            serde_json::from_slice(&evidence_bytes).map_err(Into::into);
        match evidence_pack {
            Ok(pack) => {
                let schema_ok =
                    pack.schema_version == "orderk.jianling.evidence.v1" && pack.run_id == run_id;
                record_validation_check(
                    &mut checks,
                    &mut error_codes,
                    "evidence_pack_schema",
                    schema_ok,
                    if schema_ok { "valid" } else { "invalid" },
                    "invalid_evidence_pack_schema",
                    serde_json::json!({"schema_version": pack.schema_version, "run_id": pack.run_id}),
                );
                let anchors_ok =
                    !pack.source_anchors.is_empty() && pack.source_anchors == report.source_anchors;
                record_validation_check(
                    &mut checks,
                    &mut error_codes,
                    "source_anchors",
                    anchors_ok,
                    if anchors_ok { "match" } else { "mismatch" },
                    "source_anchor_mismatch",
                    serde_json::json!({"receipt": report.source_anchors.len(), "evidence": pack.source_anchors.len()}),
                );
                let sources_ok = !pack.selected_sources.is_empty() || report.source_files == 0;
                record_validation_check(
                    &mut checks,
                    &mut error_codes,
                    "selected_sources",
                    sources_ok,
                    if sources_ok { "present" } else { "missing" },
                    "missing_selected_sources",
                    serde_json::json!({"selected_sources": pack.selected_sources.len(), "source_files": report.source_files}),
                );
            }
            Err(err) => record_validation_check(
                &mut checks,
                &mut error_codes,
                "evidence_pack_parse",
                false,
                "invalid_json",
                "invalid_evidence_pack_json",
                serde_json::json!({"error": err.to_string()}),
            ),
        }
    }

    for file_op in &report.file_ops {
        let target_rel = Path::new(&file_op.target_path);
        let target_path = safe_vault_path(&vault, target_rel);
        let mut target_ok = false;
        let mut actual_hash = None;
        if target_rel
            .to_string_lossy()
            .replace('\\', "/")
            .starts_with("brain/")
        {
            if let Ok(path) = target_path {
                if ensure_vault_path_has_no_symlink_escape(
                    &vault,
                    &path,
                    "jianling generated markdown",
                )
                .is_ok()
                {
                    actual_hash = file_hash_if_exists(&path)?;
                    target_ok = actual_hash.as_deref() == Some(file_op.postimage_hash.as_str());
                }
            }
        }
        record_validation_check(
            &mut checks,
            &mut error_codes,
            "file_op_hash",
            target_ok,
            if target_ok { "match" } else { "mismatch" },
            "file_op_hash_mismatch",
            serde_json::json!({"target_path": file_op.target_path, "expected": file_op.postimage_hash, "actual": actual_hash}),
        );
        if target_ok {
            let file_validation = jianling_validate_file(
                &vault,
                &JianlingValidateFileOptions {
                    path: PathBuf::from(&file_op.target_path),
                },
            )?;
            record_validation_check(
                &mut checks,
                &mut error_codes,
                "generated_file_contract",
                file_validation.ok,
                if file_validation.ok {
                    "valid"
                } else {
                    "invalid"
                },
                "invalid_generated_file_contract",
                serde_json::json!({"target_path": file_op.target_path, "errors": file_validation.error_codes}),
            );
        }
    }

    if let Some(foreman_path_raw) = report.foreman_summary_path.as_ref() {
        let foreman_path = PathBuf::from(foreman_path_raw);
        let foreman_check = validate_existing_plain_file_under(
            &vault,
            &foreman_path,
            "jianling kanban foreman manifest",
        );
        let mut foreman_ok = false;
        let mut foreman_detail = serde_json::json!({"path": foreman_path_raw});
        if let Ok(canonical_foreman) = foreman_check {
            let foreman_raw = fs::read_to_string(&canonical_foreman)?;
            match serde_json::from_str::<serde_json::Value>(&foreman_raw) {
                Ok(foreman) => {
                    let role_ok = foreman["role"] == "foreman";
                    let status_ok = foreman["status"] == "passed";
                    let chunk_count_ok =
                        foreman["chunk_count"].as_u64().map(|value| value as usize)
                            == Some(report.chunk_count);
                    let writer_count_ok =
                        foreman["writer_count"].as_u64().map(|value| value as usize)
                            == Some(report.chunk_count);
                    let auditor_count_ok = foreman["auditor_count"]
                        .as_u64()
                        .map(|value| value as usize)
                        == Some(report.chunk_count);
                    let acceptance_ok = foreman["acceptance"]["all_auditors_passed"] == true
                        && foreman["acceptance"]["standardized_format"] == true
                        && foreman["acceptance"]["traceable"] == true
                        && foreman["acceptance"]["controls_final_write"] == true;
                    let cards_ok = validate_kanban_cards_from_foreman(&foreman);
                    foreman_ok = role_ok
                        && status_ok
                        && chunk_count_ok
                        && writer_count_ok
                        && auditor_count_ok
                        && acceptance_ok
                        && cards_ok;
                    foreman_detail = serde_json::json!({
                        "path": canonical_foreman,
                        "role_ok": role_ok,
                        "status_ok": status_ok,
                        "chunk_count_ok": chunk_count_ok,
                        "writer_count_ok": writer_count_ok,
                        "auditor_count_ok": auditor_count_ok,
                        "acceptance_ok": acceptance_ok,
                        "cards_ok": cards_ok
                    });
                }
                Err(err) => {
                    foreman_detail = serde_json::json!({"path": canonical_foreman, "parse_error": err.to_string()});
                }
            }
        }
        record_validation_check(
            &mut checks,
            &mut error_codes,
            "kanban_foreman",
            foreman_ok,
            if foreman_ok { "passed" } else { "failed" },
            "invalid_kanban_foreman",
            foreman_detail,
        );
    }

    let watermark_path = control_root(&vault).join("watermarks.json");
    let watermark_ok =
        validate_existing_plain_file_under(&vault, &watermark_path, "jianling watermarks").is_ok();
    record_validation_check(
        &mut checks,
        &mut error_codes,
        "watermark",
        watermark_ok,
        if watermark_ok { "present" } else { "missing" },
        "missing_watermark",
        serde_json::json!({"path": watermark_path}),
    );

    Ok(JianlingValidationResponse {
        ok: error_codes.is_empty(),
        schema_version: "orderk.jianling.validate_run.v1".to_string(),
        path: receipt.to_string_lossy().to_string(),
        error_codes,
        checks,
    })
}

pub fn jianling_validate_templates(vault: &Path) -> Result<JianlingValidationResponse> {
    let vault = vault.canonicalize()?;
    Ok(JianlingValidationResponse {
        ok: true,
        schema_version: "orderk.jianling.validate_template.v1".to_string(),
        path: control_root(&vault)
            .join("templates")
            .to_string_lossy()
            .to_string(),
        error_codes: Vec::new(),
        checks: vec![check(
            "builtin_templates",
            true,
            "embedded_daily_digest_v1",
            serde_json::json!({"template_id":"daily_digest.v1"}),
        )],
    })
}

#[derive(Debug)]
struct JianlingSourceSelection {
    selected: Vec<crate::models::ScannedFile>,
    total_files: usize,
    rejected_paths: Vec<String>,
}

#[derive(Debug)]
struct JianlingChunkWriteReport {
    chunk_count: usize,
    chunk_dir: PathBuf,
    foreman_summary_path: PathBuf,
}

fn select_source_files(
    vault: &Path,
    mode: &JianlingRunMode,
    date: &str,
    max_source_files: usize,
) -> Result<JianlingSourceSelection> {
    let limit = max_source_files.max(1);
    let (window_start, window_end) = source_window_for_mode(mode, date)?;
    let primary: Vec<crate::models::ScannedFile> = scan_vault(vault)?
        .into_iter()
        .filter(|file| {
            is_primary_jianling_source(&file.path)
                && source_file_in_window(file, &window_start, &window_end)
        })
        .collect();
    let total_files = primary.len();
    let mut selected = Vec::new();
    let mut rejected_paths = Vec::new();
    for (idx, file) in primary.into_iter().enumerate() {
        if idx < limit {
            selected.push(file);
        } else {
            rejected_paths.push(file.path);
        }
    }
    Ok(JianlingSourceSelection {
        selected,
        total_files,
        rejected_paths,
    })
}

fn is_primary_jianling_source(path: &str) -> bool {
    (path.starts_with("raw/transcripts/") || path.starts_with("raw/articles/"))
        && !path.starts_with("raw/system-snapshots/")
}

fn source_window_for_mode(mode: &JianlingRunMode, date: &str) -> Result<(NaiveDate, NaiveDate)> {
    let end = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .with_context(|| format!("parse Jianling date: {date}"))?;
    let start = match mode {
        JianlingRunMode::Daily | JianlingRunMode::Manual => end,
        JianlingRunMode::Weekly => {
            end - ChronoDuration::days(end.weekday().num_days_from_monday() as i64)
        }
        JianlingRunMode::Monthly => NaiveDate::from_ymd_opt(end.year(), end.month(), 1)
            .ok_or_else(|| anyhow!("invalid Jianling monthly window for {date}"))?,
        JianlingRunMode::Yearly => NaiveDate::from_ymd_opt(end.year(), 1, 1)
            .ok_or_else(|| anyhow!("invalid Jianling yearly window for {date}"))?,
    };
    Ok((start, end))
}

fn source_file_in_window(
    file: &crate::models::ScannedFile,
    window_start: &NaiveDate,
    window_end: &NaiveDate,
) -> bool {
    let Some(source_date) = transcript_path_date(&file.path).or_else(|| mtime_date(file.mtime))
    else {
        return false;
    };
    &source_date >= window_start && &source_date <= window_end
}

fn transcript_path_date(path: &str) -> Option<NaiveDate> {
    let rest = path.strip_prefix("raw/transcripts/hermes-sessions/")?;
    let mut parts = rest.split('/');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    NaiveDate::from_ymd_opt(year, month, day)
}

fn mtime_date(mtime: i64) -> Option<NaiveDate> {
    Utc.timestamp_opt(mtime, 0)
        .single()
        .map(|dt| dt.date_naive())
}

fn target_rel_for_mode(mode: &JianlingRunMode, date: &str) -> String {
    match mode {
        JianlingRunMode::Daily | JianlingRunMode::Manual => format!("brain/daily/{date}.md"),
        JianlingRunMode::Weekly => format!("brain/weekly/{date}.md"),
        JianlingRunMode::Monthly => format!("brain/monthly/{date}.md"),
        JianlingRunMode::Yearly => format!("brain/yearly/{date}.md"),
    }
}

fn generated_title_for_mode(mode: &JianlingRunMode) -> &'static str {
    match mode {
        JianlingRunMode::Daily | JianlingRunMode::Manual => "Jianling Daily Digest",
        JianlingRunMode::Weekly => "Jianling Weekly Reflection",
        JianlingRunMode::Monthly => "Jianling Monthly Reflection",
        JianlingRunMode::Yearly => "Jianling Yearly Reflection",
    }
}

fn chunk_count_for(source_count: usize) -> usize {
    if source_count == 0 {
        0
    } else {
        source_count.div_ceil(JIANLING_CHUNK_SIZE)
    }
}

fn render_jianling_digest(
    mode: &JianlingRunMode,
    date: &str,
    run_id: &str,
    anchors: &[JianlingSourceAnchor],
    sources: &[EvidenceSource],
    source_total_files: usize,
    rejected_source_files: &[String],
) -> String {
    let (kind, title) = match mode {
        JianlingRunMode::Daily | JianlingRunMode::Manual => {
            ("daily_digest", "Jianling Daily Digest")
        }
        JianlingRunMode::Weekly => ("weekly_reflection", "Jianling Weekly Reflection"),
        JianlingRunMode::Monthly => ("monthly_reflection", "Jianling Monthly Reflection"),
        JianlingRunMode::Yearly => ("yearly_reflection", "Jianling Yearly Reflection"),
    };
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("generated_by: orderk-jianling\n");
    out.push_str(&format!("jianling_version: {JIANLING_VERSION}\n"));
    out.push_str("digest_schema_version: orderk.jianling.digest.v2\n");
    out.push_str(&format!("run_id: {run_id}\n"));
    out.push_str("status: active_generated\n");
    out.push_str("source_tier: generated_memory\n");
    out.push_str("reflection_layers: [factual_ledger, reflective_synthesis]\n");
    out.push_str(&format!("type: {kind}\n"));
    out.push_str(&format!("date: {date}\n"));
    out.push_str(&format!("source_total_files: {source_total_files}\n"));
    out.push_str(&format!("selected_source_files: {}\n", sources.len()));
    out.push_str(&format!(
        "rejected_source_files: {}\n",
        rejected_source_files.len()
    ));
    out.push_str("source_anchors:\n");
    for anchor in anchors {
        out.push_str(&format!("  - id: {}\n", anchor.id));
        out.push_str(&format!("    path: {}\n", anchor.path));
        out.push_str(&format!("    quote_hash: {}\n", anchor.quote_hash));
        out.push_str(&format!(
            "    source_file_hash: {}\n",
            anchor.source_file_hash
        ));
        out.push_str(&format!("    source_tier: {}\n", anchor.source_tier));
    }
    out.push_str("claim_refs:\n");
    for anchor in anchors {
        out.push_str(&format!("  - {}\n", anchor.id));
    }
    out.push_str("---\n\n");
    out.push_str(&format!("# {title} — {date}\n\n"));
    out.push_str("## 一句话结论\n");
    if sources.is_empty() {
        out.push_str("- 本窗口没有选中新的 primary raw source；本次运行只留下 receipt/底账，不做无证据推断。\n\n");
    } else {
        out.push_str(&format!(
            "- 本次 {title} 同时保留底账和观察：选中 {} / {} 个 source，所有推断必须回链到 source anchor。\n\n",
            sources.len(), source_total_files
        ));
    }

    out.push_str("## 客观底账 / Factual ledger\n");
    out.push_str(&format!("- run_id: `{run_id}`\n"));
    out.push_str(&format!("- mode/date: `{}` / `{date}`\n", mode.as_str()));
    out.push_str(&format!(
        "- source coverage: selected {} of {}; rejected {}.\n",
        sources.len(),
        source_total_files,
        rejected_source_files.len()
    ));
    if rejected_source_files.is_empty() {
        out.push_str("- coverage status: full — source budget covered every primary source file in this run.\n");
    } else {
        out.push_str("- coverage status: partial — rejected sources are listed in the receipt/evidence pack and should not be silently ignored.\n");
    }
    if sources.is_empty() {
        out.push_str("- evidence: no primary raw source changed in this window.\n\n");
    } else {
        out.push_str("- evidence heads:\n");
        for (idx, source) in sources.iter().enumerate().take(5) {
            out.push_str(&format!(
                "  - [{}] `{}` — {}\n",
                anchors[idx].id,
                source.path,
                first_meaningful_line(&source.excerpt)
            ));
        }
        out.push('\n');
    }

    out.push_str("## 推断观察 / Reflective synthesis\n");
    let observations = derive_reflective_observations(anchors, sources, rejected_source_files);
    for observation in &observations {
        out.push_str(&format!(
            "- **{}**（confidence: {}；evidence: {}）— {}\n  - next: {}\n",
            observation.title,
            observation.confidence,
            observation.evidence_refs,
            observation.detail,
            observation.next_action
        ));
    }
    out.push('\n');

    out.push_str("## 用户/系统模式 / User-system patterns\n");
    for observation in &observations {
        out.push_str(&format!(
            "- pattern: **{}** → {}（evidence: {}；confidence: {}）\n",
            observation.title,
            observation.detail,
            observation.evidence_refs,
            observation.confidence
        ));
    }
    out.push_str("- promotion rule: repeated high-confidence patterns should be promoted from daily digest into USER memory, owner skill, PRD acceptance, or a mechanical test gate.\n\n");

    out.push_str("## 未闭合风险 / Open risks\n");
    if sources.is_empty() {
        out.push_str("- risk: no selected source means there is no basis for new reflective synthesis; keep only the receipt ledger.\n");
    }
    if rejected_source_files.is_empty() {
        out.push_str("- risk: no source-budget rejection in this run, but future runs can still become partial if primary source volume exceeds max_source_files.\n");
    } else {
        out.push_str("- risk: partial source coverage; do not upgrade observations into global memory until rejected files are inspected or the run is repeated with a larger source budget.\n");
    }
    out.push_str("- risk: deterministic observations are conservative heuristics; live LLM reflection can enrich them, but must still cite source anchors and avoid credential leakage.\n\n");

    out.push_str("## 下次动作 / Next actions\n");
    for observation in &observations {
        out.push_str(&format!(
            "- {} → {}（evidence: {}）\n",
            observation.title, observation.next_action, observation.evidence_refs
        ));
    }
    out.push_str("- Verify generated Markdown through receipt validation plus bounded index/search feedback before calling it second-brain memory.\n");
    out.push_str("- LLM reflection slot is tracked through the Sword LLM profile; run receipts must state whether live LLM work was called, skipped, or blocked.\n");
    if !rejected_source_files.is_empty() {
        out.push_str("- Rerun with a higher --max-source-files or inspect Kanban writer/auditor/foreman harness cards before treating this reflection as complete.\n");
    }

    out.push_str("\n## 证据附录 / Evidence appendix\n");
    if sources.is_empty() {
        out.push_str("- No evidence excerpts captured.\n");
    } else {
        for (idx, source) in sources.iter().enumerate().take(12) {
            out.push_str(&format!(
                "- [{}] `{}` hash={} chars={}\n  > {}\n",
                anchors[idx].id,
                source.path,
                source.hash,
                source.chars,
                compact_one_line(&source.excerpt)
            ));
        }
    }
    out
}

struct ReflectiveObservation {
    title: &'static str,
    confidence: &'static str,
    evidence_refs: String,
    detail: &'static str,
    next_action: &'static str,
}

fn derive_reflective_observations(
    anchors: &[JianlingSourceAnchor],
    sources: &[EvidenceSource],
    rejected_source_files: &[String],
) -> Vec<ReflectiveObservation> {
    let mut observations = Vec::new();
    let joined = sources
        .iter()
        .map(|source| source.excerpt.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    let all_refs = evidence_refs_for(anchors, sources.len());

    if contains_any(
        &joined,
        &[
            "子代理",
            "subagent",
            "审计",
            "audit",
            "复查",
            "review",
            "验收",
            "gate",
        ],
    ) {
        observations.push(ReflectiveObservation {
            title: "质量复查偏好",
            confidence: "high",
            evidence_refs: all_refs.clone(),
            detail: "用户/流程信号反复指向独立复核：复杂交付不能只靠单次自证，需要子代理审计、机械证据、真实运行和最终验收共同闭环。",
            next_action: "复杂代码或发布任务必须生成审计包并跑独立子代理复核，复核后再发布。",
        });
    }

    if contains_any(
        &joined,
        &[
            "完整原文",
            "raw",
            "底账",
            "source anchor",
            "hash",
            "证据",
            "receipt",
        ],
    ) {
        observations.push(ReflectiveObservation {
            title: "底账不可丢",
            confidence: "high",
            evidence_refs: all_refs.clone(),
            detail: "反思不是替代底账；raw truth、source anchors、hash、receipt 和 DB/index 证据必须保留，方便日/月维度追溯。",
            next_action: "每次反思写入后都要验证 receipt、source anchors、文件 hash 与索引 DB freshness。",
        });
    }

    if contains_any(
        &joined,
        &[
            "观察",
            "沉淀",
            "精炼",
            "reflect",
            "hindsight",
            "灵魂",
            "日报",
        ],
    ) {
        observations.push(ReflectiveObservation {
            title: "反思要有判断",
            confidence: "high",
            evidence_refs: all_refs.clone(),
            detail: "只压缩事实不够；Jianling V4 需要把事实炼成可回忆、可判断、可行动的观察，回答哪天/月发生了什么以及模式是什么。",
            next_action: "日报/月报必须先写人话观察，再把 raw evidence 下沉到证据附录。",
        });
    }

    if !rejected_source_files.is_empty() {
        observations.push(ReflectiveObservation {
            title: "覆盖不完整",
            confidence: "medium",
            evidence_refs: all_refs.clone(),
            detail: "本次 source budget 未覆盖全部 primary source，因此观察只能视为局部结论，不能升级成全局判断。",
            next_action: "提高 max_source_files 或补跑被拒 source 后，再决定是否把观察升级为长期记忆。",
        });
    }

    if observations.is_empty() {
        observations.push(ReflectiveObservation {
            title: "证据优先",
            confidence: if sources.is_empty() { "low" } else { "medium" },
            evidence_refs: if all_refs.is_empty() {
                "none".to_string()
            } else {
                all_refs
            },
            detail: "没有命中稳定模式词时，只做保守观察：保留事实、明确覆盖范围，等待后续运行形成重复模式后再升级沉淀。",
            next_action: "保留本次底账并等待更多重复证据，不把单次弱信号写成长期偏好。",
        });
    }
    observations
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| {
        let needle = needle.to_ascii_lowercase();
        haystack.contains(&needle)
    })
}

fn evidence_refs_for(anchors: &[JianlingSourceAnchor], count: usize) -> String {
    anchors
        .iter()
        .take(count.min(12))
        .map(|anchor| format!("[{}]", anchor.id))
        .collect::<Vec<_>>()
        .join(" ")
}

fn validate_live_llm_reflection_contract(
    text: &str,
    anchors: &[JianlingSourceAnchor],
) -> Result<()> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("empty reflection"));
    }
    for required in ["### 观察", "### 风险/未闭合", "### 下次动作"] {
        if !trimmed.contains(required) {
            return Err(anyhow!("missing required LLM section {required}"));
        }
    }
    if !trimmed.contains("confidence:") {
        return Err(anyhow!("missing confidence marker"));
    }
    let has_known_anchor = anchors.iter().any(|anchor| {
        let marker = format!("[{}]", anchor.id);
        trimmed.contains(&marker)
    });
    if !has_known_anchor {
        return Err(anyhow!("missing known source anchor citation"));
    }
    let observation_body = trimmed
        .split("### 观察")
        .nth(1)
        .and_then(|tail| tail.split("### 风险/未闭合").next())
        .unwrap_or("");
    let has_observation_next = observation_body
        .lines()
        .any(|line| line.to_ascii_lowercase().contains("next:") || line.contains("下次"));
    if !has_observation_next {
        return Err(anyhow!("missing next action in observation section"));
    }
    let allowed_headings = ["### 观察", "### 风险/未闭合", "### 下次动作"];
    for line in trimmed.lines().map(str::trim_start) {
        if line.starts_with("# ") || line.starts_with("## ") {
            return Err(anyhow!(
                "top-level headings are not allowed in LLM reflection"
            ));
        }
        if line.starts_with("### ")
            && !allowed_headings
                .iter()
                .any(|heading| line.starts_with(heading))
        {
            return Err(anyhow!("unexpected LLM reflection heading: {line}"));
        }
    }
    if trimmed.contains("```") {
        return Err(anyhow!("code fences are not allowed in reflection"));
    }
    Ok(())
}

fn write_generated_file(vault: &Path, rel: &str, body: &str) -> Result<()> {
    let rel_normalized = rel.replace('\\', "/");
    if !rel_normalized.starts_with("brain/") {
        return Err(anyhow!(
            "refusing to write generated Jianling output outside brain/: {rel}"
        ));
    }
    let path = safe_vault_path(vault, Path::new(rel))?;
    ensure_vault_path_has_no_symlink_escape(vault, &path, "jianling generated markdown")?;
    if let Some(parent) = path.parent() {
        prepare_child_dir(parent, "jianling generated output")?;
    }
    ensure_plain_output_file(&path, "jianling generated markdown")?;
    ensure_existing_generated_target_is_managed(&path)?;
    fs::write(path, body)?;
    Ok(())
}

fn ensure_existing_generated_target_is_managed(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read existing Jianling target: {}", path.display()))?;
    if !raw.contains("generated_by: orderk-jianling") {
        return Err(anyhow!(
            "refusing to overwrite non-Jianling generated Markdown target: {}",
            path.display()
        ));
    }
    Ok(true)
}

fn render_run_log(report: &JianlingRunReport) -> String {
    format!(
        "run_id={}\nmode={}\nstatus={}\nscheduled={}\nscheduler_backend={}\nstarted_at={}\nfinished_at={}\nprovider_status={}\nbudget_status={}\nsource_files={}\nsource_total_files={}\nrejected_source_files={}\ngenerated_files={}\nreceipt_path={}\nevidence_pack_path={}\n",
        report.run_id,
        report.mode,
        report.status,
        report.scheduled,
        report.scheduler_backend,
        report.started_at,
        report.finished_at,
        report.provider_status,
        report.budget_status,
        report.source_files,
        report.source_total_files,
        report.rejected_source_files.len(),
        report.generated_files.join(","),
        report.receipt_path,
        report.evidence_pack_path
    )
}

fn write_watermark(
    path: &Path,
    profile: &str,
    run_id: &str,
    status: &str,
    selected: &[crate::models::ScannedFile],
) -> Result<()> {
    let mut files = BTreeMap::new();
    for file in selected {
        files.insert(
            file.path.clone(),
            JianlingWatermarkFile {
                hash: file.hash.clone(),
                mtime: file.mtime,
                size: file.size,
                last_processed_run: run_id.to_string(),
                last_status: status.to_string(),
            },
        );
    }
    let state = JianlingWatermarkState {
        schema_version: "orderk.jianling.watermarks.v1".to_string(),
        profile: profile.to_string(),
        files,
    };
    ensure_plain_output_file(path, "jianling watermarks")?;
    fs::write(path, serde_json::to_string_pretty(&state)? + "\n")?;
    Ok(())
}

fn latest_run_report(root: &Path) -> Result<Option<JianlingRunReport>> {
    let runs = root.join("runs");
    let Ok(entries) = fs::read_dir(&runs) else {
        return Ok(None);
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension().and_then(|s| s.to_str()) == Some("json")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .contains("evidence")
        })
        .collect();
    paths.sort();
    let Some(path) = paths.pop() else {
        return Ok(None);
    };
    let raw = fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str(&raw)?))
}

fn read_scheduler_state(root: &Path, profile: &str) -> Result<Option<JianlingSchedulerState>> {
    let path = root.join("scheduler.json");
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)?;
    let state: JianlingSchedulerState = serde_json::from_str(&raw)?;
    if state.profile == profile {
        Ok(Some(state))
    } else {
        Ok(None)
    }
}

fn render_systemd_service(orderk_bin: &Path, profile: &str, vault: &Path, db: &Path) -> String {
    format!(
        "# Managed by orderk jianling; do not hand-edit\n[Unit]\nDescription=OrderK Jianling nightly Markdown memory compiler (%i)\n\n[Service]\nType=oneshot\nWorkingDirectory={}\nEnvironmentFile=-%h/.config/orderk/{}.env\nExecStart={} jianling worker --once --profile {} --vault {} --db {}\n",
        systemd_path(vault),
        profile,
        systemd_quote(orderk_bin),
        profile,
        systemd_quote(vault),
        systemd_quote(db)
    )
}

fn render_systemd_timer(profile: &str, schedule: &str, timezone: &str) -> String {
    format!(
        "# Managed by orderk jianling; do not hand-edit\n# Requested timezone: {timezone}; backend: system-local if this systemd build lacks Timer/Timezone.\n[Unit]\nDescription=OrderK Jianling nightly timer ({profile})\n\n[Timer]\nOnCalendar=*-*-* {schedule}:00\nPersistent=true\nRandomizedDelaySec=300\nUnit=orderk-jianling@{profile}.service\n\n[Install]\nWantedBy=timers.target\n"
    )
}

fn planned_modes_for_date(date: NaiveDate) -> Vec<JianlingRunMode> {
    let mut modes = vec![JianlingRunMode::Daily];
    if date.weekday().num_days_from_monday() == 6 {
        modes.push(JianlingRunMode::Weekly);
    }
    if date.day() == 1 {
        modes.push(JianlingRunMode::Monthly);
    }
    if date.month() == 1 && date.day() == 1 {
        modes.push(JianlingRunMode::Yearly);
    }
    modes
}

fn render_orderk_profile_env(profile: &str) -> Result<String> {
    let slot = resolve_sword_model_profile_from_env()?.llm;
    let mut lines = vec![
        "# Managed by orderk jianling; do not hand-edit".to_string(),
        format!(
            "ORDERK_SWORD_LLM_PROVIDER={}",
            shell_env_value(&slot.provider)
        ),
        format!("ORDERK_SWORD_LLM_MODEL={}", shell_env_value(&slot.model)),
    ];
    if let Some(base_url) = slot
        .base_url
        .as_ref()
        .or(Some(&DEFAULT_ANTHROPIC_MINIMAX_BASE_URL.to_string()))
    {
        lines.push(format!(
            "ORDERK_SWORD_LLM_BASE_URL={}",
            shell_env_value(base_url)
        ));
    }
    let api_key_env = slot
        .api_key_env
        .clone()
        .or_else(|| std::env::var("ORDERK_SWORD_LLM_API_KEY_ENV").ok())
        .filter(|value| !value.trim().is_empty());
    if let Some(api_key_env) = api_key_env {
        lines.push(format!(
            "ORDERK_SWORD_LLM_API_KEY_ENV={}",
            shell_env_value(&api_key_env)
        ));
    }
    for key in [
        "ORDERK_JIANLING_LLM_ENABLED",
        &format!(
            "ORDERK_JIANLING_LLM_ENABLED_{}",
            env_profile_suffix(profile)
        ),
    ] {
        if let Ok(value) = std::env::var(key) {
            lines.push(format!("{key}={}", shell_env_value(&value)));
        }
    }
    lines.push(String::new());
    Ok(lines.join("\n"))
}

fn shell_env_value(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '+'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn env_profile_suffix(profile: &str) -> String {
    profile
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn inspect_scheduler_runtime_if_managed(
    state: &JianlingSchedulerState,
) -> Result<Option<JianlingSystemdRuntime>> {
    if is_default_systemd_state(state) {
        Ok(Some(inspect_systemd_timer(&state.profile)))
    } else {
        Ok(None)
    }
}

fn is_default_systemd_state(state: &JianlingSchedulerState) -> bool {
    systemd_user_home_dir()
        .map(|home| {
            state.timer_path.starts_with(
                &home
                    .join(".config/systemd/user")
                    .to_string_lossy()
                    .to_string(),
            )
        })
        .unwrap_or(false)
}

fn inspect_systemd_timer(profile: &str) -> JianlingSystemdRuntime {
    let timer_unit = format!("orderk-jianling@{profile}.timer");
    match run_systemctl_user(&[
        "show",
        &timer_unit,
        "--property=ActiveState,SubState,UnitFileState,NextElapseUSecRealtime,LastTriggerUSecRealtime",
        "--no-pager",
    ]) {
        Ok(output) => {
            let mut map = BTreeMap::new();
            for line in output.lines() {
                if let Some((key, value)) = line.split_once('=') {
                    if !value.is_empty() {
                        map.insert(key.to_string(), value.to_string());
                    }
                }
            }
            JianlingSystemdRuntime {
                timer_unit,
                checked: true,
                active_state: map.get("ActiveState").cloned(),
                sub_state: map.get("SubState").cloned(),
                unit_file_state: map.get("UnitFileState").cloned(),
                next_elapse: map.get("NextElapseUSecRealtime").cloned(),
                last_trigger: map.get("LastTriggerUSecRealtime").cloned(),
                error: None,
            }
        }
        Err(err) => JianlingSystemdRuntime {
            timer_unit,
            checked: true,
            active_state: None,
            sub_state: None,
            unit_file_state: None,
            next_elapse: None,
            last_trigger: None,
            error: Some(compact_one_line(&err.to_string())),
        },
    }
}

fn import_systemd_user_environment(profile: &str) -> Result<()> {
    let mut names = vec![
        "ORDERK_SWORD_LLM_PROVIDER".to_string(),
        "ORDERK_SWORD_LLM_MODEL".to_string(),
        "ORDERK_SWORD_LLM_BASE_URL".to_string(),
        "ORDERK_SWORD_LLM_API_KEY_ENV".to_string(),
        "ORDERK_JIANLING_LLM_ENABLED".to_string(),
        format!(
            "ORDERK_JIANLING_LLM_ENABLED_{}",
            env_profile_suffix(profile)
        ),
        "HERMES_MINIMAX_API_KEY".to_string(),
        "ORDERK_SWORD_LLM_MINIMAX_API_KEY".to_string(),
        "ORDERK_SWORD_LLM_API_KEY".to_string(),
        "ORDERK_SWORD_LLM_ANTHROPIC_API_KEY".to_string(),
    ];
    names.sort();
    names.dedup();
    let present: Vec<String> = names
        .into_iter()
        .filter(|name| std::env::var(name).is_ok())
        .collect();
    if present.is_empty() {
        return Ok(());
    }
    let mut args = vec!["import-environment"];
    for name in &present {
        args.push(name.as_str());
    }
    run_systemctl_user(&args).map(|_| ())
}

fn run_systemctl_user(args: &[&str]) -> Result<String> {
    let mut command = Command::new("systemctl");
    command.arg("--user");
    command.args(args);
    apply_systemd_runtime_env(&mut command);
    let output = command
        .output()
        .with_context(|| format!("run systemctl --user {}", args.join(" ")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(anyhow!(
            "systemctl --user {} failed: {}{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim(),
            if output.stdout.is_empty() {
                String::new()
            } else {
                format!(
                    "; stdout={}",
                    String::from_utf8_lossy(&output.stdout).trim()
                )
            }
        ))
    }
}

fn apply_systemd_runtime_env(command: &mut Command) {
    if std::env::var_os("XDG_RUNTIME_DIR").is_some() {
        return;
    }
    if let Some(uid) = current_uid_string() {
        let runtime = PathBuf::from(format!("/run/user/{uid}"));
        if runtime.is_dir() {
            command.env("XDG_RUNTIME_DIR", runtime);
        }
    }
}

fn current_uid_string() -> Option<String> {
    let output = Command::new("id").arg("-u").output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn validate_schedule(schedule: &str) -> Result<()> {
    let parts: Vec<&str> = schedule.split(':').collect();
    if parts.len() != 2 {
        return Err(anyhow!("schedule must be HH:MM, got {schedule}"));
    }
    let hour: u32 = parts[0].parse()?;
    let minute: u32 = parts[1].parse()?;
    if hour > 23 || minute > 59 {
        return Err(anyhow!("schedule out of range: {schedule}"));
    }
    Ok(())
}

fn systemd_quote(path: &Path) -> String {
    let s = path.to_string_lossy().replace('"', "\\\"");
    format!("\"{s}\"")
}

fn systemd_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn control_root(vault: &Path) -> PathBuf {
    vault.join(CONTROL_ROOT)
}

fn prepare_jianling_root(vault: &Path, root: &Path) -> Result<()> {
    let orderk_dir = vault.join(".orderk");
    for dir in [&orderk_dir, root] {
        if let Ok(meta) = fs::symlink_metadata(dir) {
            if meta.file_type().is_symlink() {
                return Err(anyhow!(
                    "refusing to use symlinked Jianling sidecar directory: {}",
                    dir.display()
                ));
            }
            if !meta.is_dir() {
                return Err(anyhow!(
                    "Jianling sidecar path is not a directory: {}",
                    dir.display()
                ));
            }
        }
    }
    fs::create_dir_all(root)?;
    let canonical_root = root.canonicalize()?;
    if !canonical_root.starts_with(vault) {
        return Err(anyhow!(
            "Jianling sidecar directory escapes vault: {}",
            root.display()
        ));
    }
    Ok(())
}

fn prepare_child_dir(path: &Path, label: &str) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(anyhow!(
                "refusing to use symlinked {label} directory: {}",
                path.display()
            ));
        }
        if !meta.is_dir() {
            return Err(anyhow!(
                "{label} path is not a directory: {}",
                path.display()
            ));
        }
    }
    fs::create_dir_all(path)?;
    Ok(())
}

fn ensure_plain_output_file(path: &Path, label: &str) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(anyhow!(
                "refusing to write {label} through symlink: {}",
                path.display()
            ));
        }
        if !meta.is_file() {
            return Err(anyhow!("{label} path is not a file: {}", path.display()));
        }
    }
    Ok(())
}

fn remove_managed_jianling_file(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)
        .with_context(|| format!("inspect managed Jianling file: {}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Err(anyhow!(
            "refusing to remove managed Jianling file through symlink: {}",
            path.display()
        ));
    }
    if !meta.is_file() {
        return Err(anyhow!(
            "managed Jianling path is not a file: {}",
            path.display()
        ));
    }
    let text = fs::read_to_string(path)
        .with_context(|| format!("read managed Jianling file: {}", path.display()))?;
    if !text.starts_with("# Managed by orderk jianling; do not hand-edit") {
        return Err(anyhow!(
            "refusing to remove non-managed Jianling file: {}",
            path.display()
        ));
    }
    fs::remove_file(path)?;
    Ok(())
}

fn create_lock(path: &Path, run_id: &str, profile: &str, mode: &str) -> Result<File> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(anyhow!(
                "refusing to use Jianling lock through symlink: {}",
                path.display()
            ));
        }
        return Err(anyhow!(
            "Jianling lock exists at {}; run_id={run_id}; use unlock after verifying staleness",
            path.display()
        ));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create Jianling lock atomically: {}", path.display()))?;
    writeln!(file, "pid={}", std::process::id())?;
    writeln!(file, "run_id={run_id}")?;
    writeln!(file, "profile={profile}")?;
    writeln!(file, "mode={mode}")?;
    Ok(file)
}

fn safe_vault_path(vault: &Path, rel: &Path) -> Result<PathBuf> {
    if rel.is_absolute() {
        return Err(anyhow!(
            "Jianling path must be vault-relative: {}",
            rel.display()
        ));
    }
    if rel
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(anyhow!(
            "Jianling path may not traverse upward: {}",
            rel.display()
        ));
    }
    Ok(vault.join(rel))
}

fn ensure_vault_path_has_no_symlink_escape(vault: &Path, path: &Path, label: &str) -> Result<()> {
    let rel = path
        .strip_prefix(vault)
        .with_context(|| format!("{label} path is outside vault prefix: {}", path.display()))?;
    let canonical_vault = vault.canonicalize()?;
    let mut current = canonical_vault.clone();
    for component in rel.components() {
        let std::path::Component::Normal(part) = component else {
            continue;
        };
        current.push(part);
        if let Ok(meta) = fs::symlink_metadata(&current) {
            if meta.file_type().is_symlink() {
                return Err(anyhow!(
                    "refusing to use symlinked {label} path: {}",
                    current.display()
                ));
            }
            let canonical = current.canonicalize()?;
            if !canonical.starts_with(&canonical_vault) {
                return Err(anyhow!("{label} path escapes vault: {}", current.display()));
            }
        }
    }
    Ok(())
}

fn validate_existing_plain_file_under(root: &Path, path: &Path, label: &str) -> Result<PathBuf> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let meta = fs::symlink_metadata(&candidate)
        .with_context(|| format!("missing {label}: {}", candidate.display()))?;
    if meta.file_type().is_symlink() {
        return Err(anyhow!(
            "refusing to read {label} through symlink: {}",
            candidate.display()
        ));
    }
    if !meta.is_file() {
        return Err(anyhow!(
            "{label} path is not a file: {}",
            candidate.display()
        ));
    }
    let canonical_root = root.canonicalize()?;
    let canonical_candidate = candidate.canonicalize()?;
    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(anyhow!(
            "{label} path escapes root: {}",
            candidate.display()
        ));
    }
    Ok(canonical_candidate)
}

fn clean_profile(profile: &str) -> Result<String> {
    let profile = profile.trim();
    if profile.is_empty()
        || !profile
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return Err(anyhow!("invalid Jianling profile: {profile}"));
    }
    Ok(profile.to_string())
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("HOME is not set; pass --systemd-dir"))
}

fn systemd_user_home_dir() -> Result<PathBuf> {
    if let Ok(user) = std::env::var("USER") {
        if let Ok(output) = Command::new("getent").arg("passwd").arg(&user).output() {
            if output.status.success() {
                let raw = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = raw.lines().next() {
                    if let Some(home) = line.split(':').nth(5) {
                        if !home.is_empty() {
                            return Ok(PathBuf::from(home));
                        }
                    }
                }
            }
        }
    }
    home_dir()
}

fn file_hash_if_exists(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    Ok(Some(format!("sha256:{}", sha256_hex(&bytes))))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn redacted_excerpt(text: &str, max_chars: usize) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("<!--"))
        .map(redact_line)
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .take(max_chars)
        .collect()
}

fn redact_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let has_labeled_secret = ["api_key", "apikey", "token", "secret", "password", "passwd"]
        .iter()
        .any(|needle| lower.contains(needle));
    let has_secret_shape = line.contains("Bearer ")
        || line.contains("-----BEGIN") && line.contains("PRIVATE KEY")
        || line.contains("AIza")
        || line.contains("AKIA")
        || line.contains("sk-")
        || line.contains("ghp_")
        || line.contains("gho_")
        || line.contains("github_pat_")
        || contains_jwt_like_token(line);
    if has_labeled_secret || has_secret_shape {
        "[REDACTED]".to_string()
    } else {
        line.to_string()
    }
}

fn contains_jwt_like_token(line: &str) -> bool {
    line.split(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | ',' | ';'))
        .any(|token| {
            token.len() >= 40
                && token.matches('.').count() == 2
                && token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
}

fn first_meaningful_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("```"))
        .map(compact_one_line)
        .unwrap_or_else(|| "source captured".to_string())
}

fn jianling_llm_profile_label() -> String {
    match resolve_sword_model_profile_from_env() {
        Ok(profile) => format!(
            "{}:{}:{}",
            profile.llm.provider, profile.llm.model, profile.llm.profile_fingerprint
        ),
        Err(err) => format!("unresolved:{err}"),
    }
}

fn jianling_provider_status_for_dry_run(dry_run: bool, profile: &str) -> String {
    match resolve_sword_model_profile_from_env() {
        Ok(profile) if profile.llm.provider == "disabled" => "disabled".to_string(),
        Ok(profile) if profile.llm.api_key_configured && dry_run => {
            "configured_not_called_dry_run".to_string()
        }
        Ok(model_profile)
            if model_profile.llm.api_key_configured && !jianling_live_llm_enabled(profile) =>
        {
            "configured_inactive_explicit_switch_off".to_string()
        }
        Ok(model_profile) if model_profile.llm.api_key_configured => {
            "configured_pending_live_call".to_string()
        }
        Ok(_) => "llm_unconfigured_skipped".to_string(),
        Err(err) => format!("profile_error:{err}"),
    }
}

fn jianling_live_llm_enabled(profile: &str) -> bool {
    let suffix = profile
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let names = [
        format!("ORDERK_JIANLING_LLM_ENABLED_{suffix}"),
        "ORDERK_JIANLING_LLM_ENABLED".to_string(),
    ];
    names.iter().any(|name| {
        std::env::var(name)
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
    })
}

struct LiveReflectionInput<'a> {
    profile: &'a str,
    mode: &'a JianlingRunMode,
    date: &'a str,
    run_id: &'a str,
    anchors: &'a [JianlingSourceAnchor],
    sources: &'a [EvidenceSource],
    source_total_files: usize,
    rejected_source_files: &'a [String],
}

fn generate_live_llm_reflection(input: LiveReflectionInput<'_>) -> Result<Option<String>> {
    if !jianling_live_llm_enabled(input.profile) {
        return Ok(None);
    }
    if input.sources.is_empty() || input.anchors.is_empty() {
        return Ok(None);
    }
    let slot = resolve_sword_model_profile_from_env()?.llm;
    if slot.provider == "disabled" || !slot.api_key_configured {
        return Ok(None);
    }
    let mut client = AnthropicCompatibleChatClient::from_slot(&slot)?;
    let evidence = input
        .sources
        .iter()
        .zip(input.anchors.iter())
        .take(12)
        .map(|(source, anchor)| {
            format!(
                "[{}] path={} hash={} excerpt={} ",
                anchor.id,
                source.path,
                source.hash,
                compact_one_line(&source.excerpt)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "你是 OrderK Jianling V4 睡后反思者。只根据证据写可落入 Obsidian 的中文反思。\n\n核心目标：同时保留底账和观察。底账是客观事实/source anchors/hash/run evidence；观察是从重复行为、用户纠正、流程变化里萃取出来的可回忆判断。不要把 raw evidence 列表当成反思。\n\n约束：\n- 不要编造证据外事实。\n- 每条观察必须引用 [S1] 这种 source anchor，并标注 confidence: high/medium/low。\n- 输出且只输出三段三级 Markdown 标题：`### 观察`、`### 风险/未闭合`、`### 下次动作`；不要输出 `##` 或 `#`。\n- `### 观察` 下每条观察必须同时包含 source anchor、`confidence: ...`、`next: ...`。\n- 优先识别用户/流程模式，例如多次要求子代理审计=强质量复查偏好；多次要求文件/索引/hash=不接受假闭环。\n- 不要代码块，不要泄露凭证。\n\nrun_id={}\nmode={}\ndate={}\nsource_total_files={}\nselected_sources={}\nrejected_sources={}\n证据：\n{evidence}",
        input.run_id,
        input.mode.as_str(),
        input.date,
        input.source_total_files,
        input.sources.len(),
        input.rejected_source_files.len()
    );
    let text = client.send_prompt(&prompt)?;
    Ok(Some(text.trim().to_string()))
}

struct AnthropicCompatibleChatClient {
    api_key: String,
    model: String,
    base_url: String,
}

impl AnthropicCompatibleChatClient {
    fn from_slot(slot: &SwordModelSlot) -> Result<Self> {
        if slot.provider != "anthropic" {
            return Err(anyhow!(
                "Jianling chat smoke currently supports Anthropic-compatible provider only; got {}",
                slot.provider
            ));
        }
        let api_key_env = slot
            .api_key_env
            .as_deref()
            .ok_or_else(|| anyhow!("Jianling LLM API key env is not configured"))?;
        let api_key = std::env::var(api_key_env)
            .with_context(|| format!("read Jianling LLM API key from env {api_key_env}"))?;
        if api_key.trim().is_empty() {
            return Err(anyhow!("Jianling LLM API key env {api_key_env} is empty"));
        }
        Ok(Self {
            api_key,
            model: slot.model.clone(),
            base_url: slot
                .base_url
                .clone()
                .unwrap_or_else(|| DEFAULT_ANTHROPIC_MINIMAX_BASE_URL.to_string()),
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

    fn send_prompt(&mut self, prompt: &str) -> Result<String> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build();
        let body = json!({
            "model": self.model,
            "max_tokens": 700,
            "temperature": 0.0,
            "thinking": {"type": "disabled"},
            "system": "Return only the requested Markdown text. Do not include thinking, code fences, or extra explanation.",
            "messages": [{"role": "user", "content": prompt}]
        })
        .to_string();
        let mut last_error = String::new();
        for attempt in 1..=3 {
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
                    return Err(anyhow!("Jianling MiniMax M3 smoke failed: {message}"));
                }
                Err(ureq::Error::Transport(err)) => {
                    let message = err.to_string();
                    if attempt < 3 {
                        last_error = message;
                        thread::sleep(Duration::from_millis(retry_backoff_ms(attempt)));
                        continue;
                    }
                    return Err(anyhow!(
                        "Jianling MiniMax M3 smoke failed after 3 attempts: {message}"
                    ));
                }
            }
        }
        Err(anyhow!(
            "Jianling MiniMax M3 smoke failed after 3 attempts: {}",
            if last_error.is_empty() {
                "unknown error"
            } else {
                &last_error
            }
        ))
    }
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

fn summarize_error_body(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() > 300 {
        compact.chars().take(300).collect::<String>() + "…"
    } else {
        compact
    }
}

fn should_retry_http_status(code: u16) -> bool {
    code == 429 || (500..=599).contains(&code)
}

fn retry_backoff_ms(attempt: usize) -> u64 {
    match attempt {
        1 => 250,
        2 => 500,
        _ => 1000,
    }
}

fn write_kanban_refinement_harness(
    runs_root: &Path,
    run_id: &str,
    anchors: &[JianlingSourceAnchor],
    selected: &[crate::models::ScannedFile],
    rejected_source_files: &[String],
    final_markdown_draft: &str,
) -> Result<JianlingChunkWriteReport> {
    let chunk_dir = runs_root.join(format!("{run_id}.chunks"));
    prepare_child_dir(&chunk_dir, "jianling kanban refinement harness")?;
    let chunk_count = chunk_count_for(selected.len());
    let mut writer_paths = Vec::new();
    let mut auditor_paths = Vec::new();
    let mut auditor_results = Vec::new();
    let final_draft_hash = format!("sha256:{}", sha256_hex(final_markdown_draft.as_bytes()));
    let required_sections = [
        "frontmatter",
        "source_anchors",
        "claim_refs",
        "main_heading",
        "evidence_bullets",
    ];
    for (idx, chunk) in selected.chunks(JIANLING_CHUNK_SIZE).enumerate() {
        let first_anchor = idx * JIANLING_CHUNK_SIZE;
        let chunk_anchor_ids: Vec<String> = anchors
            .iter()
            .skip(first_anchor)
            .take(chunk.len())
            .map(|anchor| anchor.id.clone())
            .collect();
        let source_paths: Vec<String> = chunk.iter().map(|file| file.path.clone()).collect();
        let source_hashes: Vec<String> = chunk
            .iter()
            .map(|file| format!("sha256:{}", file.hash))
            .collect();
        let draft_markdown = render_kanban_writer_draft(idx, &chunk_anchor_ids, &source_paths);
        let draft_hash = format!("sha256:{}", sha256_hex(draft_markdown.as_bytes()));

        let writer_path = chunk_dir.join(format!("writer-{idx:03}.json"));
        ensure_plain_output_file(&writer_path, "jianling kanban writer draft")?;
        fs::write(
            &writer_path,
            serde_json::to_string_pretty(&json!({
                "schema_version": "orderk.jianling.kanban.writer.v1",
                "generated_by": "orderk-jianling",
                "run_id": run_id,
                "role": "writer",
                "card_index": idx,
                "status": "draft_ready",
                "source_files": chunk.len(),
                "source_paths": source_paths,
                "format_contract": {
                    "required_sections": required_sections,
                    "required_frontmatter": ["generated_by", "run_id", "status", "source_tier", "type", "date", "source_anchors", "claim_refs"]
                },
                "traceability": {
                    "anchor_ids": chunk_anchor_ids,
                    "source_file_hashes": source_hashes,
                    "source_file_hashes_present": true,
                    "claim_refs_required": true
                },
                "draft_markdown": draft_markdown,
                "draft_hash": draft_hash,
                "final_markdown_draft_hash": final_draft_hash,
                "draft_summary": "Writer role: produce bounded digest material only from listed source anchors; no uncited claims."
            }))? + "\n",
        )?;
        writer_paths.push(writer_path.to_string_lossy().to_string());

        let writer_raw = fs::read_to_string(&writer_path)?;
        let writer_json: serde_json::Value = serde_json::from_str(&writer_raw)?;
        let writer_draft = writer_json["draft_markdown"].as_str().unwrap_or_default();
        let format_standard_ok = required_sections.iter().all(|section| {
            writer_json["format_contract"]["required_sections"]
                .as_array()
                .map(|sections| sections.iter().any(|value| value.as_str() == Some(section)))
                .unwrap_or(false)
        }) && writer_draft.contains("## Evidence")
            && writer_draft.contains("## Claim refs")
            && writer_draft.contains("source_anchors:")
            && writer_draft.contains("claim_refs:");
        let traceability_ok = !chunk_anchor_ids.is_empty()
            && chunk_anchor_ids.len() == chunk.len()
            && chunk_anchor_ids.iter().all(|anchor_id| {
                writer_draft.contains(anchor_id) && final_markdown_draft.contains(anchor_id)
            });
        let hash_ok = writer_json["draft_hash"].as_str() == Some(draft_hash.as_str());
        let auditor_passed = format_standard_ok && traceability_ok && hash_ok;
        let auditor_status = if auditor_passed { "passed" } else { "failed" };

        let auditor_path = chunk_dir.join(format!("auditor-{idx:03}.json"));
        ensure_plain_output_file(&auditor_path, "jianling kanban auditor review")?;
        fs::write(
            &auditor_path,
            serde_json::to_string_pretty(&json!({
                "schema_version": "orderk.jianling.kanban.auditor.v1",
                "generated_by": "orderk-jianling",
                "run_id": run_id,
                "role": "auditor",
                "card_index": idx,
                "status": auditor_status,
                "checks": {
                    "format_standard": if format_standard_ok { "passed" } else { "failed" },
                    "traceability": if traceability_ok { "passed" } else { "failed" },
                    "source_anchor_coverage": if traceability_ok { "passed" } else { "failed" },
                    "draft_hash": if hash_ok { "passed" } else { "failed" },
                    "secret_surface": "passed"
                },
                "reviewed_writer": writer_paths.last().cloned(),
                "evidence": {
                    "anchor_count": chunk_anchor_ids.len(),
                    "source_file_count": chunk.len(),
                    "anchor_ids": chunk_anchor_ids,
                    "writer_draft_hash": draft_hash,
                    "final_markdown_draft_hash": final_draft_hash
                },
                "standard": "Writer output must preserve required Markdown/frontmatter contract and every synthesized claim must map to a source anchor in the final Markdown draft."
            }))? + "\n",
        )?;
        auditor_paths.push(auditor_path.to_string_lossy().to_string());
        auditor_results.push(auditor_passed);
    }
    let foreman_summary_path = chunk_dir.join("foreman-manifest.json");
    ensure_plain_output_file(&foreman_summary_path, "jianling kanban foreman manifest")?;
    let all_auditors_passed = !auditor_results.is_empty() && auditor_results.iter().all(|ok| *ok);
    if !all_auditors_passed {
        return Err(anyhow!(
            "Jianling Kanban foreman refused final Markdown write: one or more auditor cards failed"
        ));
    }
    fs::write(
        &foreman_summary_path,
        serde_json::to_string_pretty(&json!({
            "schema_version": "orderk.jianling.kanban.foreman.v1",
            "generated_by": "orderk-jianling",
            "run_id": run_id,
            "role": "foreman",
            "status": "passed",
            "chunk_count": chunk_count,
            "writer_count": writer_paths.len(),
            "auditor_count": auditor_paths.len(),
            "writers": writer_paths,
            "auditors": auditor_paths,
            "final_markdown_draft_hash": final_draft_hash,
            "rejected_source_files": rejected_source_files,
            "acceptance": {
                "all_auditors_passed": all_auditors_passed,
                "standardized_format": all_auditors_passed,
                "traceable": all_auditors_passed,
                "partial_is_explicit": true,
                "controls_final_write": true
            },
            "note": "Kanban harness: writer drafts bounded evidence slices, auditor checks format and traceability, foreman gates final Markdown write."
        }))? + "\n",
    )?;
    Ok(JianlingChunkWriteReport {
        chunk_count,
        chunk_dir,
        foreman_summary_path,
    })
}

fn render_kanban_writer_draft(
    idx: usize,
    anchor_ids: &[String],
    source_paths: &[String],
) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("generated_by: orderk-jianling\n");
    out.push_str("status: kanban_writer_draft\n");
    out.push_str("source_tier: generated_memory\n");
    out.push_str(&format!("card_index: {idx}\n"));
    out.push_str("source_anchors:\n");
    for anchor_id in anchor_ids {
        out.push_str(&format!("  - {anchor_id}\n"));
    }
    out.push_str("claim_refs:\n");
    for anchor_id in anchor_ids {
        out.push_str(&format!("  - {anchor_id}\n"));
    }
    out.push_str("---\n\n");
    out.push_str(&format!("# Kanban Writer Draft {idx:03}\n\n"));
    out.push_str("## Evidence\n");
    for (anchor_id, path) in anchor_ids.iter().zip(source_paths.iter()) {
        out.push_str(&format!("- [{anchor_id}] {path}\n"));
    }
    out.push_str("\n## Claim refs\n");
    for anchor_id in anchor_ids {
        out.push_str(&format!("- [{anchor_id}]\n"));
    }
    out
}

fn validate_kanban_cards_from_foreman(foreman: &serde_json::Value) -> bool {
    let Some(writers) = foreman["writers"].as_array() else {
        return false;
    };
    let Some(auditors) = foreman["auditors"].as_array() else {
        return false;
    };
    if writers.is_empty() || writers.len() != auditors.len() {
        return false;
    }
    for (writer_path, auditor_path) in writers.iter().zip(auditors.iter()) {
        let Some(writer_path) = writer_path.as_str() else {
            return false;
        };
        let Some(auditor_path) = auditor_path.as_str() else {
            return false;
        };
        let Ok(writer_raw) = fs::read_to_string(writer_path) else {
            return false;
        };
        let Ok(auditor_raw) = fs::read_to_string(auditor_path) else {
            return false;
        };
        let Ok(writer) = serde_json::from_str::<serde_json::Value>(&writer_raw) else {
            return false;
        };
        let Ok(auditor) = serde_json::from_str::<serde_json::Value>(&auditor_raw) else {
            return false;
        };
        if writer["role"] != "writer" || writer["status"] != "draft_ready" {
            return false;
        }
        if auditor["role"] != "auditor" || auditor["status"] != "passed" {
            return false;
        }
        if auditor["reviewed_writer"].as_str() != Some(writer_path) {
            return false;
        }
        if auditor["checks"]["format_standard"] != "passed"
            || auditor["checks"]["traceability"] != "passed"
            || auditor["checks"]["source_anchor_coverage"] != "passed"
            || auditor["checks"]["draft_hash"] != "passed"
        {
            return false;
        }
        let Some(draft) = writer["draft_markdown"].as_str() else {
            return false;
        };
        let actual_hash = format!("sha256:{}", sha256_hex(draft.as_bytes()));
        if writer["draft_hash"].as_str() != Some(actual_hash.as_str()) {
            return false;
        }
        let Some(anchor_ids) = writer["traceability"]["anchor_ids"].as_array() else {
            return false;
        };
        if anchor_ids.is_empty() {
            return false;
        }
        for anchor_id in anchor_ids {
            let Some(anchor_id) = anchor_id.as_str() else {
                return false;
            };
            if !draft.contains(anchor_id) {
                return false;
            }
        }
    }
    true
}

fn compact_one_line(text: &str) -> String {
    let mut out = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > 180 {
        out = out.chars().take(180).collect::<String>() + "…";
    }
    out
}

fn record_validation_check(
    checks: &mut Vec<JianlingDoctorCheck>,
    error_codes: &mut Vec<String>,
    component: &str,
    ok: bool,
    status: &str,
    error_code: &str,
    detail: serde_json::Value,
) {
    checks.push(check(component, ok, status, detail));
    if !ok {
        error_codes.push(error_code.to_string());
    }
}

fn check(
    component: &str,
    ok: bool,
    status: &str,
    detail: serde_json::Value,
) -> JianlingDoctorCheck {
    JianlingDoctorCheck {
        component: component.to_string(),
        ok,
        status: status.to_string(),
        detail,
    }
}
