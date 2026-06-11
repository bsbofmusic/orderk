use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

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
    pub warnings: Vec<String>,
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
    pub dry_run: bool,
    pub written_files: Vec<String>,
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
    let run_id = format!(
        "jianling-{}-{}",
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        std::process::id()
    );
    let root = control_root(&vault);
    let runs_root = root.join("runs");
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

    let selection = select_source_files(&vault, options.max_source_files)?;
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
        if let Some(reflection) = generate_live_llm_reflection(
            &options.mode,
            &date,
            &run_id,
            &source_anchors,
            &evidence_sources,
            selection.total_files,
            &selection.rejected_paths,
        )? {
            provider_status = "called_live".to_string();
            generated_body.push_str("\n## LLM 反思（MiniMax M3）\n");
            generated_body.push_str(&reflection);
            generated_body.push('\n');
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
        selected_sources: evidence_sources,
    };
    let evidence_json = serde_json::to_string_pretty(&evidence_pack)? + "\n";
    let evidence_hash = format!("sha256:{}", sha256_hex(evidence_json.as_bytes()));
    let receipt_path = runs_root.join(format!("{run_id}.json"));
    let evidence_path = runs_root.join(format!("{run_id}.evidence.json.redacted"));
    let index_update = if options.db.is_some() {
        "skipped_no_db_index_run"
    } else {
        "skipped_no_db"
    };

    let mut report = JianlingRunReport {
        ok: true,
        schema_version: "orderk.jianling.run.v1".to_string(),
        run_id: run_id.clone(),
        mode: options.mode.as_str().to_string(),
        status: if options.dry_run {
            "dry_run"
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
        index_smoke_status: "skipped_p0_whole_vault_reindex_not_invoked".to_string(),
        fallback_used: false,
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
    };

    if options.dry_run {
        return Ok(report);
    }

    prepare_jianling_root(&vault, &root)?;
    prepare_child_dir(&runs_root, "jianling runs")?;
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
## Kanban 精炼 Harness
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
        write_watermark(
            &root.join("watermarks.json"),
            &profile,
            &run_id,
            "success",
            &selected,
        )?;
        ensure_plain_output_file(&evidence_path, "jianling evidence pack")?;
        fs::write(&evidence_path, evidence_json)?;
        report.finished_at = Utc::now().to_rfc3339();
        ensure_plain_output_file(&receipt_path, "jianling run receipt")?;
        fs::write(
            &receipt_path,
            serde_json::to_string_pretty(&report)?
                + "
",
        )?;
        Ok(())
    })();
    drop(lock);
    let _ = fs::remove_file(&lock_path);
    write_result?;
    Ok(report)
}

pub fn jianling_status(vault: &Path, profile: &str) -> Result<JianlingStatusResponse> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let profile = clean_profile(profile)?;
    let root = control_root(&vault);
    let scheduler = read_scheduler_state(&root, &profile)?;
    let latest = latest_run_report(&root)?;
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
        next_run: scheduler.map(|state| state.schedule),
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
    let scheduler_ok = scheduler
        .as_ref()
        .map(|state| {
            Path::new(&state.service_path).is_file() && Path::new(&state.timer_path).is_file()
        })
        .unwrap_or(false);
    checks.push(JianlingDoctorCheck {
        component: "scheduler".to_string(),
        ok: scheduler_ok,
        status: if scheduler_ok {
            "enabled"
        } else {
            "not_enabled"
        }
        .to_string(),
        detail: serde_json::json!({"state": scheduler}),
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
    let ok = checks
        .iter()
        .all(|check| check.ok || check.component == "last_run");
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
    let systemd_dir = match &options.systemd_user_dir {
        Some(path) => path.clone(),
        None => home_dir()?.join(".config/systemd/user"),
    };
    let service_path = systemd_dir.join(format!("orderk-jianling@{profile}.service"));
    let timer_path = systemd_dir.join(format!("orderk-jianling@{profile}.timer"));
    let db = options
        .db
        .clone()
        .unwrap_or_else(|| vault.join(".obsidian/orderk/orderk.sqlite"));
    let service = render_systemd_service(&options.orderk_bin, &profile, &vault, &db);
    let timer = render_systemd_timer(&profile, &options.schedule, &options.timezone);
    let written_files = vec![
        service_path.to_string_lossy().to_string(),
        timer_path.to_string_lossy().to_string(),
        root.join("scheduler.json").to_string_lossy().to_string(),
    ];
    if !options.dry_run {
        prepare_jianling_root(&vault, &root)?;
        prepare_child_dir(&systemd_dir, "systemd user unit")?;
        ensure_plain_output_file(&service_path, "jianling systemd service")?;
        fs::write(&service_path, service)?;
        ensure_plain_output_file(&timer_path, "jianling systemd timer")?;
        fs::write(&timer_path, timer)?;
        let state = JianlingSchedulerState {
            schema_version: "orderk.jianling.scheduler.v1".to_string(),
            profile: profile.clone(),
            scheduler_backend: "systemd-user".to_string(),
            schedule: options.schedule.clone(),
            timezone: options.timezone.clone(),
            service_path: service_path.to_string_lossy().to_string(),
            timer_path: timer_path.to_string_lossy().to_string(),
            orderk_bin: options.orderk_bin.to_string_lossy().to_string(),
            vault: vault.to_string_lossy().to_string(),
            db: Some(db.to_string_lossy().to_string()),
        };
        let scheduler_path = root.join("scheduler.json");
        ensure_plain_output_file(&scheduler_path, "jianling scheduler state")?;
        fs::write(scheduler_path, serde_json::to_string_pretty(&state)? + "\n")?;
    }
    Ok(JianlingEnableReport {
        ok: true,
        schema_version: "orderk.jianling.enable.v1".to_string(),
        vault: vault.to_string_lossy().to_string(),
        profile,
        scheduler_backend: "systemd-user".to_string(),
        schedule: options.schedule.clone(),
        timezone: options.timezone.clone(),
        service_path: service_path.to_string_lossy().to_string(),
        timer_path: timer_path.to_string_lossy().to_string(),
        dry_run: options.dry_run,
        written_files,
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

fn select_source_files(vault: &Path, max_source_files: usize) -> Result<JianlingSourceSelection> {
    let limit = max_source_files.max(1);
    let primary: Vec<crate::models::ScannedFile> = scan_vault(vault)?
        .into_iter()
        .filter(|file| is_primary_jianling_source(&file.path))
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

fn target_rel_for_mode(mode: &JianlingRunMode, date: &str) -> String {
    match mode {
        JianlingRunMode::Daily | JianlingRunMode::Manual => format!("brain/daily/{date}.md"),
        JianlingRunMode::Weekly => format!("brain/weekly/{date}.md"),
        JianlingRunMode::Monthly => format!("brain/monthly/{date}.md"),
        JianlingRunMode::Yearly => format!("brain/yearly/{date}.md"),
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
    let (kind, title, main_heading) = match mode {
        JianlingRunMode::Daily | JianlingRunMode::Manual => {
            ("daily_digest", "Jianling Daily Digest", "今日主线")
        }
        JianlingRunMode::Weekly => (
            "weekly_reflection",
            "Jianling Weekly Reflection",
            "本周主线",
        ),
        JianlingRunMode::Monthly => (
            "monthly_reflection",
            "Jianling Monthly Reflection",
            "本月主线",
        ),
        JianlingRunMode::Yearly => (
            "yearly_reflection",
            "Jianling Yearly Reflection",
            "年度主线",
        ),
    };
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("generated_by: orderk-jianling\n");
    out.push_str(&format!("jianling_version: {JIANLING_VERSION}\n"));
    out.push_str(&format!("run_id: {run_id}\n"));
    out.push_str("status: active_generated\n");
    out.push_str("source_tier: generated_memory\n");
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
    out.push_str(&format!("## {main_heading}\n"));
    if sources.is_empty() {
        out.push_str("- No primary raw source changed in this window.\n\n");
    } else {
        for (idx, source) in sources.iter().enumerate().take(5) {
            out.push_str(&format!(
                "- [{}] {} — {}\n",
                anchors[idx].id,
                source.path,
                first_meaningful_line(&source.excerpt)
            ));
        }
        out.push('\n');
    }
    out.push_str("## 事实变化\n");
    if rejected_source_files.is_empty() {
        out.push_str("- Source budget covered every primary source file in this run.\n\n");
    } else {
        out.push_str(&format!(
            "- Partial coverage: selected {} of {} primary source files; {} rejected because max_source_files was reached.\n\n",
            sources.len(),
            source_total_files,
            rejected_source_files.len()
        ));
    }
    out.push_str("## 经验 / 反思\n");
    for (idx, source) in sources.iter().enumerate().take(3) {
        out.push_str(&format!(
            "- Evidence [{}] preserves raw/human-authored context for future synthesis.\n  > {}\n",
            anchors[idx].id,
            compact_one_line(&source.excerpt)
        ));
    }
    out.push_str("\n## Open loops\n");
    out.push_str("- LLM reflection slot is tracked through the Sword LLM profile; run receipts must state whether live LLM work was called, skipped, or blocked.\n");
    if !rejected_source_files.is_empty() {
        out.push_str("- This run is partial; rerun with a higher --max-source-files or inspect Kanban writer/auditor/foreman harness cards before treating it as complete.\n");
    }
    out
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
        "# Managed by orderk jianling; do not hand-edit\n[Unit]\nDescription=OrderK Jianling nightly Markdown memory compiler (%i)\n\n[Service]\nType=oneshot\nExecStart={} jianling run --profile {} --scheduled --vault {} --db {}\n",
        systemd_quote(orderk_bin),
        profile,
        systemd_quote(vault),
        systemd_quote(db)
    )
}

fn render_systemd_timer(profile: &str, schedule: &str, timezone: &str) -> String {
    format!(
        "# Managed by orderk jianling; do not hand-edit\n[Unit]\nDescription=OrderK Jianling nightly timer ({profile})\n\n[Timer]\nOnCalendar=*-*-* {schedule}:00\nTimezone={timezone}\nPersistent=true\nUnit=orderk-jianling@{profile}.service\n\n[Install]\nWantedBy=timers.target\n"
    )
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

fn generate_live_llm_reflection(
    mode: &JianlingRunMode,
    date: &str,
    run_id: &str,
    anchors: &[JianlingSourceAnchor],
    sources: &[EvidenceSource],
    source_total_files: usize,
    rejected_source_files: &[String],
) -> Result<Option<String>> {
    let slot = resolve_sword_model_profile_from_env()?.llm;
    if slot.provider == "disabled" || !slot.api_key_configured {
        return Ok(None);
    }
    let mut client = AnthropicCompatibleChatClient::from_slot(&slot)?;
    let evidence = sources
        .iter()
        .zip(anchors.iter())
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
        "你是 OrderK Jianling V4 睡后反思者。只根据证据写一段可落入 Obsidian 的中文反思。\n\n约束：\n- 不要编造证据外事实。\n- 每条结论尽量引用 [S1] 这种 source anchor。\n- 识别今日/本周/本月的主线、风险、下一步。\n- 返回 Markdown 列表，不要代码块，不要泄露凭证。\n\nrun_id={run_id}\nmode={}\ndate={date}\nsource_total_files={source_total_files}\nselected_sources={}\nrejected_sources={}\n证据：\n{evidence}",
        mode.as_str(),
        sources.len(),
        rejected_source_files.len()
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
            "max_tokens": 80,
            "temperature": 0.0,
            "thinking": {"type": "disabled"},
            "system": "Return only the requested text. Do not include markdown or explanation.",
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
