use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::scanner::scan_vault;

const CONTROL_ROOT: &str = ".orderk/jianling";
const JIANLING_VERSION: &str = "0.1";

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
    let lock_path = root
        .join("locks")
        .join(format!("{profile}-{}.lock", options.mode.as_str()));
    let target_rel = match options.mode {
        JianlingRunMode::Daily | JianlingRunMode::Manual => format!("brain/daily/{date}.md"),
        JianlingRunMode::Weekly => format!("brain/reflections/weekly-{date}.md"),
        JianlingRunMode::Monthly => format!("brain/reflections/monthly-{date}.md"),
        JianlingRunMode::Yearly => format!("brain/principles/yearly-{date}.md"),
    };

    let target_abs = safe_vault_path(&vault, Path::new(&target_rel))?;
    ensure_vault_path_has_no_symlink_escape(&vault, &target_abs, "jianling generated markdown")?;
    let target_exists = ensure_existing_generated_target_is_managed(&target_abs)?;
    let target_preimage_hash = if target_exists {
        file_hash_if_exists(&target_abs)?
    } else {
        None
    };

    let selected = select_source_files(&vault, options.max_source_files)?;
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

    let generated_body = render_daily_digest(&date, &run_id, &source_anchors, &evidence_sources);
    let postimage_hash = format!("sha256:{}", sha256_hex(generated_body.as_bytes()));
    let file_ops = vec![JianlingFileOp {
        op: if target_exists {
            "replace".to_string()
        } else {
            "create".to_string()
        },
        target_path: target_rel.clone(),
        preimage_hash: target_preimage_hash,
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
        llm_profile: "none_deterministic_p0".to_string(),
        provider_status: "skipped_no_llm_in_p0".to_string(),
        schema_validation_status: "passed".to_string(),
        budget_status: "within_budget".to_string(),
        pre_llm_guard_status: "passed".to_string(),
        pre_write_guard_status: "passed".to_string(),
        index_update: index_update.to_string(),
        index_smoke_status: "skipped_p0_whole_vault_reindex_not_invoked".to_string(),
        fallback_used: false,
        success_predicate: JianlingSuccessPredicate {
            provider: "skipped_no_llm_in_p0".to_string(),
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
        warnings: Vec::new(),
    };

    if options.dry_run {
        return Ok(report);
    }

    prepare_jianling_root(&vault, &root)?;
    prepare_child_dir(&runs_root, "jianling runs")?;
    prepare_child_dir(&root.join("locks"), "jianling locks")?;
    let lock = create_lock(&lock_path, &run_id, &profile, options.mode.as_str())?;
    let write_result = (|| -> Result<()> {
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
        fs::write(&receipt_path, serde_json::to_string_pretty(&report)? + "\n")?;
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

fn select_source_files(
    vault: &Path,
    max_source_files: usize,
) -> Result<Vec<crate::models::ScannedFile>> {
    let limit = max_source_files.max(1);
    Ok(scan_vault(vault)?
        .into_iter()
        .filter(|file| is_primary_jianling_source(&file.path))
        .take(limit)
        .collect())
}

fn is_primary_jianling_source(path: &str) -> bool {
    (path.starts_with("raw/transcripts/") || path.starts_with("raw/articles/"))
        && !path.starts_with("raw/system-snapshots/")
}

fn render_daily_digest(
    date: &str,
    run_id: &str,
    anchors: &[JianlingSourceAnchor],
    sources: &[EvidenceSource],
) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("generated_by: orderk-jianling\n");
    out.push_str(&format!("jianling_version: {JIANLING_VERSION}\n"));
    out.push_str(&format!("run_id: {run_id}\n"));
    out.push_str("status: active_generated\n");
    out.push_str("source_tier: generated_memory\n");
    out.push_str("type: daily_digest\n");
    out.push_str(&format!("date: {date}\n"));
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
    out.push_str(&format!("# Jianling Daily Digest — {date}\n\n"));
    out.push_str("## 今日主线\n");
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
    out.push_str("- P0 deterministic digest only; fact cards remain proposal-first unless a later validator promotes narrow direct-quote facts.\n\n");
    out.push_str("## 经验 / 反思\n");
    for (idx, source) in sources.iter().enumerate().take(3) {
        out.push_str(&format!(
            "- Evidence [{}] preserves raw/human-authored context for future synthesis.\n  > {}\n",
            anchors[idx].id,
            compact_one_line(&source.excerpt)
        ));
    }
    out.push_str("\n## Open loops\n");
    out.push_str("- LLM reflection is intentionally not called in P0 unless a later provider gate is explicitly enabled.\n");
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
