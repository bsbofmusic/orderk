use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use orderk_core::{
    jianling_doctor, jianling_enable, jianling_run, jianling_status, jianling_validate_file,
    jianling_validate_run, JianlingEnableOptions, JianlingRunMode, JianlingRunOptions,
    JianlingValidateFileOptions,
};

fn temp_vault(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "orderk-jianling-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn seed_raw_dialogue(vault: &Path) {
    let raw = vault.join("raw/transcripts/hermes-sessions/2026/06/10");
    fs::create_dir_all(&raw).unwrap();
    fs::write(
        raw.join("session.md"),
        r#"# 对话原话

## Transcript

### 2026-06-10T03:00:00+08:00 — user
```text
以后 session 入库要保留去噪后的完整原文，不要压成关键词卡。
```

### 2026-06-10T03:01:00+08:00 — assistant
```text
结论：导入器只删工具噪音，保留 user/assistant 正文。
```
"#,
    )
    .unwrap();
    fs::create_dir_all(vault.join("raw/system-snapshots")).unwrap();
    fs::write(
        vault.join("raw/system-snapshots/noise.md"),
        "# noisy snapshot\n",
    )
    .unwrap();
}

#[test]
fn jianling_dry_run_reports_sources_without_writing_generated_memory() {
    let vault = temp_vault("dry-run");
    seed_raw_dialogue(&vault);

    let report = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Daily,
            dry_run: true,
            scheduled: false,
            db: None,
            date: Some("2026-06-10".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();

    assert!(report.ok);
    assert_eq!(report.schema_version, "orderk.jianling.run.v1");
    assert_eq!(report.status, "dry_run");
    assert_eq!(report.profile, "default");
    assert_eq!(report.generated_files.len(), 1);
    assert_eq!(report.generated_files[0], "brain/daily/2026-06-10.md");
    assert_eq!(report.source_files, 1, "system snapshots must be excluded");
    assert_eq!(report.generated_source_tier, "generated_memory");
    assert!(report.success_predicate.pre_write_guard == "passed");
    assert!(!vault.join("brain/daily/2026-06-10.md").exists());
    assert!(!vault.join(".orderk/jianling/watermarks.json").exists());

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_apply_writes_daily_digest_receipt_evidence_and_watermark() {
    let vault = temp_vault("apply");
    seed_raw_dialogue(&vault);

    let report = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Daily,
            dry_run: false,
            scheduled: true,
            db: Some(vault.join(".obsidian/orderk/orderk.sqlite")),
            date: Some("2026-06-10".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();

    let daily = vault.join("brain/daily/2026-06-10.md");
    let daily_text = fs::read_to_string(&daily).unwrap();
    assert!(daily_text.contains("generated_by: orderk-jianling"));
    assert!(daily_text.contains("status: active_generated"));
    assert!(daily_text.contains("source_tier: generated_memory"));
    assert!(daily_text.contains("source_anchors:"));
    assert!(daily_text.contains("session 入库要保留去噪后的完整原文"));

    assert!(Path::new(&report.receipt_path).is_file());
    assert!(Path::new(&report.evidence_pack_path).is_file());
    assert!(vault.join(".orderk/jianling/watermarks.json").is_file());
    assert_eq!(report.index_update, "skipped_no_db_index_run");
    assert!(report.lock_clean);

    let status = jianling_status(&vault, "default").unwrap();
    assert_eq!(
        status.latest_run_id.as_deref(),
        Some(report.run_id.as_str())
    );
    assert_eq!(status.last_run_status.as_deref(), Some("success"));

    let validation = jianling_validate_file(
        &vault,
        &JianlingValidateFileOptions {
            path: PathBuf::from("brain/daily/2026-06-10.md"),
        },
    )
    .unwrap();
    assert!(validation.ok, "validation should pass: {validation:#?}");

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_run_refuses_to_overwrite_human_daily_note() {
    let vault = temp_vault("human-daily-safe");
    seed_raw_dialogue(&vault);
    fs::create_dir_all(vault.join("brain/daily")).unwrap();
    let target = vault.join("brain/daily/2026-06-10.md");
    fs::write(&target, "# Human daily note\n\nDo not overwrite me.\n").unwrap();

    let err = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Daily,
            dry_run: false,
            scheduled: false,
            db: None,
            date: Some("2026-06-10".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("non-Jianling") || msg.contains("refusing to overwrite"),
        "unexpected error: {msg}"
    );
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "# Human daily note\n\nDo not overwrite me.\n"
    );

    let _ = fs::remove_dir_all(vault);
}

#[cfg(unix)]
#[test]
fn jianling_run_rejects_symlinked_generated_output_root() {
    use std::os::unix::fs as unix_fs;

    let vault = temp_vault("symlink-escape");
    seed_raw_dialogue(&vault);
    let escape = temp_vault("symlink-escape-target");
    unix_fs::symlink(&escape, vault.join("brain")).unwrap();

    let err = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Daily,
            dry_run: false,
            scheduled: false,
            db: None,
            date: Some("2026-06-10".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap_err();

    let msg = err.to_string();
    assert!(
        msg.contains("symlink") || msg.contains("escapes vault"),
        "unexpected error: {msg}"
    );
    assert!(!escape.join("daily/2026-06-10.md").exists());

    let _ = fs::remove_dir_all(vault);
    let _ = fs::remove_dir_all(escape);
}

#[test]
fn jianling_validate_run_rejects_tampered_generated_markdown_and_evidence() {
    let vault = temp_vault("tamper");
    seed_raw_dialogue(&vault);

    let report = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Daily,
            dry_run: false,
            scheduled: true,
            db: Some(vault.join(".obsidian/orderk/orderk.sqlite")),
            date: Some("2026-06-10".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();

    fs::write(
        vault.join("brain/daily/2026-06-10.md"),
        "tampered generated body\n",
    )
    .unwrap();
    fs::write(&report.evidence_pack_path, "tampered evidence\n").unwrap();

    let validation = jianling_validate_run(&vault, &report.run_id).unwrap();
    assert!(
        !validation.ok,
        "tampered run must fail validation: {validation:#?}"
    );
    assert!(validation
        .error_codes
        .contains(&"evidence_pack_hash_mismatch".to_string()));
    assert!(validation
        .error_codes
        .contains(&"file_op_hash_mismatch".to_string()));

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_disable_refuses_to_remove_non_managed_scheduler_targets() {
    let vault = temp_vault("disable-safe");
    fs::create_dir_all(vault.join(".orderk/jianling")).unwrap();
    let victim = vault.join("human-note.md");
    fs::write(&victim, "human text must survive\n").unwrap();
    fs::write(
        vault.join(".orderk/jianling/scheduler.json"),
        format!(
            "{{\n  \"schema_version\": \"orderk.jianling.scheduler.v1\",\n  \"profile\": \"default\",\n  \"enabled\": true,\n  \"scheduler_backend\": \"systemd-user\",\n  \"schedule\": \"03:30 Asia/Shanghai\",\n  \"timezone\": \"Asia/Shanghai\",\n  \"service_path\": {:?},\n  \"timer_path\": {:?},\n  \"orderk_bin\": \"/usr/bin/orderk\",\n  \"vault\": {:?},\n  \"db\": null\n}}\n",
            victim.to_string_lossy(),
            victim.to_string_lossy(),
            vault.to_string_lossy()
        ),
    )
    .unwrap();

    let err = orderk_core::jianling_disable(&vault, "default").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("non-managed") || msg.contains("managed Jianling"),
        "unexpected error: {msg}"
    );
    assert!(victim.exists(), "disable must not remove human files");

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_validator_rejects_generated_cards_without_claim_sources() {
    let vault = temp_vault("validate");
    fs::create_dir_all(vault.join("brain/reflections")).unwrap();
    fs::write(
        vault.join("brain/reflections/bad.md"),
        "---\ngenerated_by: orderk-jianling\nstatus: active_generated\nsource_tier: generated_memory\n---\n\n# Bad\nNo claim refs.\n",
    )
    .unwrap();

    let validation = jianling_validate_file(
        &vault,
        &JianlingValidateFileOptions {
            path: PathBuf::from("brain/reflections/bad.md"),
        },
    )
    .unwrap();
    assert!(!validation.ok);
    assert!(validation
        .error_codes
        .contains(&"missing_source_anchors".to_string()));
    assert!(validation
        .error_codes
        .contains(&"missing_claim_refs".to_string()));

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_enable_writes_orderk_managed_systemd_units() {
    let vault = temp_vault("enable");
    let systemd_dir = vault.join("fake-systemd");
    let bin = vault.join("bin/orderk");
    fs::create_dir_all(bin.parent().unwrap()).unwrap();
    fs::write(&bin, "#!/bin/sh\n").unwrap();

    let report = jianling_enable(
        &vault,
        &JianlingEnableOptions {
            profile: "default".to_string(),
            schedule: "03:30".to_string(),
            timezone: "Asia/Shanghai".to_string(),
            db: Some(vault.join(".obsidian/orderk/orderk.sqlite")),
            orderk_bin: bin.clone(),
            systemd_user_dir: Some(systemd_dir.clone()),
            dry_run: false,
        },
    )
    .unwrap();

    assert!(report.ok);
    assert_eq!(report.scheduler_backend, "systemd-user");
    assert!(systemd_dir
        .join("orderk-jianling@default.service")
        .is_file());
    assert!(systemd_dir.join("orderk-jianling@default.timer").is_file());
    let service = fs::read_to_string(systemd_dir.join("orderk-jianling@default.service")).unwrap();
    assert!(service.contains("# Managed by orderk jianling; do not hand-edit"));
    assert!(service.contains("jianling run --profile default --scheduled"));
    assert!(service.contains("--vault"));
    assert!(service.contains("--db"));
    let timer = fs::read_to_string(systemd_dir.join("orderk-jianling@default.timer")).unwrap();
    assert!(timer.contains("OnCalendar=*-*-* 03:30:00"));
    assert!(timer.contains("Persistent=true"));

    let doctor = jianling_doctor(&vault, "default").unwrap();
    assert!(doctor.ok, "doctor should pass after enable: {doctor:#?}");
    assert!(doctor
        .checks
        .iter()
        .any(|check| check.component == "scheduler" && check.ok));

    let _ = fs::remove_dir_all(vault);
}
