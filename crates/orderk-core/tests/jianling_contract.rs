use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use orderk_core::{
    index_vault, index_vault_with_options, jianling_chat_smoke, jianling_doctor, jianling_enable,
    jianling_run, jianling_status, jianling_validate_file, jianling_validate_run, jianling_worker,
    EmbeddingProvider, IndexOptions, JianlingEnableOptions, JianlingRunMode, JianlingRunOptions,
    JianlingValidateFileOptions, JianlingWorkerOptions, MockEmbeddingProvider, VectorBackend,
};
use rusqlite::Connection;

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

fn seed_mock_index_db(vault: &Path, db: &Path) {
    let provider = MockEmbeddingProvider::new(8);
    index_vault(
        vault,
        db,
        &provider,
        provider.dimension(),
        provider.model_id(),
        VectorBackend::Exact,
    )
    .unwrap();
}

fn seed_mock_index_db_with_options(vault: &Path, db: &Path, options: &IndexOptions) {
    let provider = MockEmbeddingProvider::new(8);
    index_vault_with_options(
        vault,
        db,
        &provider,
        provider.dimension(),
        provider.model_id(),
        VectorBackend::Exact,
        options,
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
    assert_eq!(report.index_update, "skipped_dry_run");
    assert_eq!(report.index_smoke_status, "skipped_dry_run");
    assert!(report.success_predicate.pre_write_guard == "passed");
    assert!(!vault.join("brain/daily/2026-06-10.md").exists());
    assert!(!vault.join(".orderk/jianling/watermarks.json").exists());

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_apply_writes_daily_digest_receipt_evidence_and_watermark() {
    let vault = temp_vault("apply");
    let _env = ScopedEnv::set(&[
        ("ORDERK_SWORD_EMBEDDING_PROVIDER", "mock"),
        ("ORDERK_SWORD_EMBEDDING_MODEL", "mock-8"),
        ("ORDERK_SWORD_EMBEDDING_DIM", "8"),
        ("ORDERK_SWORD_VECTOR_BACKEND", "exact"),
    ]);
    seed_raw_dialogue(&vault);

    let db = vault.join(".obsidian/orderk/orderk.sqlite");
    seed_mock_index_db(&vault, &db);
    let report = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Daily,
            dry_run: false,
            scheduled: true,
            db: Some(db.clone()),
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
    assert_eq!(report.chunking_status, "kanban_foreman_summary_written");
    assert_eq!(report.chunk_count, 1);
    let chunk_dir = Path::new(report.chunk_dir.as_ref().unwrap());
    assert!(chunk_dir.join("writer-000.json").is_file());
    assert!(chunk_dir.join("auditor-000.json").is_file());
    assert!(chunk_dir.join("foreman-manifest.json").is_file());
    assert!(Path::new(report.foreman_summary_path.as_ref().unwrap()).is_file());
    assert!(vault.join(".orderk/jianling/watermarks.json").is_file());
    assert_eq!(report.index_update, "success");
    assert_eq!(report.index_smoke_status, "passed");
    assert_eq!(report.success_predicate.index_smoke, "passed");
    let index_summary = report.index_summary.as_ref().expect("index summary");
    assert_eq!(index_summary.path, "brain/daily/2026-06-10.md");
    assert_eq!(index_summary.files, 1);
    assert!(index_summary.added + index_summary.updated + index_summary.unchanged >= 1);
    assert!(index_summary.chunks > 0);
    assert!(index_summary.embedded > 0);
    let conn = Connection::open(&db).unwrap();
    let indexed: (i64, i64, String) = conn
        .query_row(
            "SELECT size, (SELECT COUNT(*) FROM chunks WHERE file_path = files.path), hash FROM files WHERE path = ?1",
            ["brain/daily/2026-06-10.md"],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(indexed.0 as usize, report.file_ops[0].byte_count);
    assert!(indexed.1 > 0);
    assert_eq!(
        format!("sha256:{}", indexed.2),
        report.file_ops[0].postimage_hash
    );
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
fn jianling_apply_degrades_when_index_db_is_missing() {
    let vault = temp_vault("missing-index-db");
    seed_raw_dialogue(&vault);
    let db = vault.join(".obsidian/orderk/missing.sqlite");

    let report = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Daily,
            dry_run: false,
            scheduled: true,
            db: Some(db.clone()),
            date: Some("2026-06-10".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();

    assert!(!report.ok);
    assert_eq!(report.status, "degraded_index_failed");
    assert_eq!(report.index_update, "failed");
    assert_eq!(report.index_smoke_status, "skipped_index_profile_failed");
    assert!(report.index_summary.is_none());
    assert!(
        !db.exists(),
        "Jianling must not create a wrong DB during feedback"
    );
    assert!(vault.join("brain/daily/2026-06-10.md").is_file());
    let status = jianling_status(&vault, "default").unwrap();
    assert_eq!(
        status.last_run_status.as_deref(),
        Some("degraded_index_failed")
    );

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_index_feedback_reuses_existing_db_chunk_profile() {
    let vault = temp_vault("chunk-profile");
    seed_raw_dialogue(&vault);
    let db = vault.join(".obsidian/orderk/orderk.sqlite");
    let options = IndexOptions {
        chunk_max_chars: 800,
        chunk_overlap_chars: 100,
    };
    seed_mock_index_db_with_options(&vault, &db, &options);

    let report = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Daily,
            dry_run: false,
            scheduled: true,
            db: Some(db),
            date: Some("2026-06-10".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();

    assert!(
        report.ok,
        "run should pass with inherited chunk profile: {report:#?}"
    );
    assert_eq!(report.index_update, "success");
    let summary = report.index_summary.as_ref().unwrap();
    assert_eq!(summary.chunk_max_chars, 800);
    assert_eq!(summary.chunk_overlap_chars, 100);
    assert_eq!(summary.chunk_strategy, "heading_overlap");

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
    assert!(service.contains("jianling worker --once --profile default"));
    assert!(service.contains("EnvironmentFile=-%h/.config/orderk/default.env"));
    assert!(service.contains("WorkingDirectory="));
    assert!(service.contains("--vault"));
    assert!(service.contains("--db"));
    let timer = fs::read_to_string(systemd_dir.join("orderk-jianling@default.timer")).unwrap();
    assert!(timer.contains("OnCalendar=*-*-* 03:30:00"));
    assert!(timer.contains("Persistent=true"));
    assert!(timer.contains("RandomizedDelaySec=300"));

    let doctor = jianling_doctor(&vault, "default").unwrap();
    assert!(doctor.ok, "doctor should pass after enable: {doctor:#?}");
    assert!(doctor
        .checks
        .iter()
        .any(|check| check.component == "scheduler" && check.ok));

    let _ = fs::remove_dir_all(vault);
}

fn seed_many_raw_dialogues(vault: &Path, count: usize) {
    let raw = vault.join("raw/transcripts/hermes-sessions/2026/06/10");
    fs::create_dir_all(&raw).unwrap();
    for idx in 0..count {
        fs::write(
            raw.join(format!("session-{idx:03}.md")),
            format!("# Session {idx}\n\n用户说：第 {idx} 条反思素材，需要被剑灵分片处理。\n"),
        )
        .unwrap();
    }
}

#[test]
fn jianling_daily_selects_only_requested_date_window() {
    let vault = temp_vault("daily-date-window");
    let day10 = vault.join("raw/transcripts/hermes-sessions/2026/06/10");
    let day11 = vault.join("raw/transcripts/hermes-sessions/2026/06/11");
    fs::create_dir_all(&day10).unwrap();
    fs::create_dir_all(&day11).unwrap();
    fs::write(day10.join("old.md"), "# old\n\n不应该进 6/11 日反思\n").unwrap();
    fs::write(day11.join("today.md"), "# today\n\n应该进入 6/11 日反思\n").unwrap();
    fs::create_dir_all(vault.join("raw/system-snapshots/2026/06/11")).unwrap();
    fs::write(
        vault.join("raw/system-snapshots/2026/06/11/noise.md"),
        "# noisy snapshot\n",
    )
    .unwrap();

    let report = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Daily,
            dry_run: true,
            scheduled: true,
            db: None,
            date: Some("2026-06-11".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();

    assert_eq!(report.source_total_files, 1);
    assert_eq!(report.source_files, 1);
    assert_eq!(
        report.source_anchors[0].path,
        "raw/transcripts/hermes-sessions/2026/06/11/today.md"
    );

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_weekly_monthly_yearly_select_expected_date_windows() {
    let vault = temp_vault("calendar-source-window");
    for (date, name) in [
        ("2025/12/31", "old-year"),
        ("2026/01/01", "year-start"),
        ("2026/05/31", "old-month"),
        ("2026/06/01", "week-start"),
        ("2026/06/07", "sunday"),
        ("2026/06/10", "today"),
        ("2026/06/11", "future"),
    ] {
        let dir = vault.join("raw/transcripts/hermes-sessions").join(date);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{name}.md")), format!("# {name}\n")).unwrap();
    }

    let weekly = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Weekly,
            dry_run: true,
            scheduled: true,
            db: None,
            date: Some("2026-06-07".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();
    assert_eq!(weekly.source_total_files, 2);
    assert!(weekly.source_anchors.iter().all(|anchor| {
        anchor.path.contains("2026/06/01") || anchor.path.contains("2026/06/07")
    }));

    let monthly = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Monthly,
            dry_run: true,
            scheduled: true,
            db: None,
            date: Some("2026-06-10".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();
    assert_eq!(monthly.source_total_files, 3);
    assert!(monthly
        .source_anchors
        .iter()
        .all(|anchor| anchor.path.contains("2026/06/")));
    assert!(!monthly
        .source_anchors
        .iter()
        .any(|anchor| anchor.path.contains("2026/06/11")));

    let yearly = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Yearly,
            dry_run: true,
            scheduled: true,
            db: None,
            date: Some("2026-06-10".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();
    assert_eq!(yearly.source_total_files, 5);
    assert!(!yearly
        .source_anchors
        .iter()
        .any(|anchor| anchor.path.contains("2025/")));
    assert!(!yearly
        .source_anchors
        .iter()
        .any(|anchor| anchor.path.contains("2026/06/11")));

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_worker_plans_calendar_modes_without_external_cron() {
    let vault = temp_vault("worker-planner");
    let day01 = vault.join("raw/transcripts/hermes-sessions/2026/06/01");
    let day07 = vault.join("raw/transcripts/hermes-sessions/2026/06/07");
    fs::create_dir_all(&day01).unwrap();
    fs::create_dir_all(&day07).unwrap();
    fs::write(day01.join("session.md"), "# day 1\n").unwrap();
    fs::write(day07.join("session.md"), "# sunday\n").unwrap();
    let db = vault.join(".obsidian/orderk/orderk.sqlite");
    seed_mock_index_db(&vault, &db);

    let monthly = jianling_worker(
        &vault,
        &JianlingWorkerOptions {
            profile: "default".to_string(),
            db: Some(db.clone()),
            date: Some("2026-06-01".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();
    assert_eq!(monthly.modes_planned, vec!["daily", "monthly"]);
    assert_eq!(monthly.runs.len(), 2);
    assert!(monthly.ok, "monthly worker should pass: {monthly:#?}");
    assert_eq!(monthly.status, "success");
    assert_ne!(monthly.runs[0].run_id, monthly.runs[1].run_id);
    assert!(monthly.runs.iter().all(|run| run.index_update == "success"));
    assert!(monthly
        .runs
        .iter()
        .all(|run| run.index_smoke_status == "passed"));
    assert!(monthly.runs.iter().all(|run| run
        .index_summary
        .as_ref()
        .is_some_and(|summary| summary.files == 1)));
    assert!(vault.join("brain/daily/2026-06-01.md").is_file());
    assert!(vault.join("brain/monthly/2026-06-01.md").is_file());

    let weekly = jianling_worker(
        &vault,
        &JianlingWorkerOptions {
            profile: "default".to_string(),
            db: Some(db.clone()),
            date: Some("2026-06-07".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();
    assert_eq!(weekly.modes_planned, vec!["daily", "weekly"]);
    assert_eq!(weekly.runs.len(), 2);
    assert!(weekly.ok, "weekly worker should pass: {weekly:#?}");
    assert_eq!(weekly.status, "success");
    assert_ne!(weekly.runs[0].run_id, weekly.runs[1].run_id);
    assert!(weekly.runs.iter().all(|run| run.index_update == "success"));
    assert!(weekly
        .runs
        .iter()
        .all(|run| run.index_smoke_status == "passed"));
    assert!(weekly.runs.iter().all(|run| run
        .index_summary
        .as_ref()
        .is_some_and(|summary| summary.files == 1)));
    assert!(vault.join("brain/daily/2026-06-07.md").is_file());
    assert!(vault.join("brain/weekly/2026-06-07.md").is_file());

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_weekly_and_monthly_write_prd_paths_not_reflections_bucket() {
    let vault = temp_vault("weekly-monthly-paths");
    seed_raw_dialogue(&vault);

    let weekly = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Weekly,
            dry_run: false,
            scheduled: true,
            db: None,
            date: Some("2026-06-07".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();
    let monthly = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Monthly,
            dry_run: false,
            scheduled: true,
            db: None,
            date: Some("2026-06-10".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();

    assert_eq!(weekly.generated_files, vec!["brain/weekly/2026-06-07.md"]);
    assert_eq!(monthly.generated_files, vec!["brain/monthly/2026-06-10.md"]);
    assert!(vault.join("brain/weekly/2026-06-07.md").is_file());
    assert!(vault.join("brain/monthly/2026-06-10.md").is_file());
    assert!(!vault
        .join("brain/reflections/weekly-2026-06-07.md")
        .exists());
    assert!(!vault
        .join("brain/reflections/monthly-2026-06-10.md")
        .exists());

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_global_profile_lock_blocks_cross_mode_conflicts() {
    let vault = temp_vault("global-lock");
    seed_raw_dialogue(&vault);
    let lock_dir = vault.join(".orderk/jianling/locks");
    fs::create_dir_all(&lock_dir).unwrap();
    fs::write(
        lock_dir.join("default.lock"),
        "pid=999\nrun_id=existing\nprofile=default\nmode=daily\n",
    )
    .unwrap();

    let err = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Weekly,
            dry_run: false,
            scheduled: true,
            db: None,
            date: Some("2026-06-07".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("Jianling lock exists"),
        "unexpected error: {msg}"
    );

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_partial_large_run_writes_kanban_chunk_and_foreman_receipts() {
    let vault = temp_vault("chunked");
    seed_many_raw_dialogues(&vault, 95);

    let report = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Daily,
            dry_run: false,
            scheduled: true,
            db: None,
            date: Some("2026-06-10".to_string()),
            max_source_files: 90,
        },
    )
    .unwrap();

    assert_eq!(report.source_total_files, 95);
    assert_eq!(report.source_files, 90);
    assert_eq!(report.rejected_source_files.len(), 5);
    assert_eq!(report.budget_status, "partial_source_file_limit");
    assert_eq!(report.chunking_status, "kanban_foreman_summary_written");
    assert_eq!(report.chunk_count, 3);
    let chunk_dir = Path::new(report.chunk_dir.as_ref().unwrap());
    for idx in 0..3 {
        assert!(chunk_dir.join(format!("writer-{idx:03}.json")).is_file());
        assert!(chunk_dir.join(format!("auditor-{idx:03}.json")).is_file());
    }
    assert!(chunk_dir.join("foreman-manifest.json").is_file());
    assert!(Path::new(report.foreman_summary_path.as_ref().unwrap()).is_file());

    let writer: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(chunk_dir.join("writer-000.json")).unwrap())
            .unwrap();
    assert_eq!(writer["role"], "writer");
    assert_eq!(writer["status"], "draft_ready");
    assert!(writer["format_contract"]["required_sections"]
        .as_array()
        .unwrap()
        .contains(&serde_json::Value::String("claim_refs".to_string())));
    assert!(!writer["traceability"]["anchor_ids"]
        .as_array()
        .unwrap()
        .is_empty());

    let auditor: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(chunk_dir.join("auditor-000.json")).unwrap())
            .unwrap();
    assert_eq!(auditor["role"], "auditor");
    assert_eq!(auditor["status"], "passed");
    assert_eq!(auditor["checks"]["format_standard"], "passed");
    assert_eq!(auditor["checks"]["traceability"], "passed");
    assert_eq!(auditor["checks"]["draft_hash"], "passed");
    assert_eq!(
        auditor["reviewed_writer"],
        chunk_dir
            .join("writer-000.json")
            .to_string_lossy()
            .to_string()
    );
    assert!(writer["draft_markdown"]
        .as_str()
        .unwrap()
        .contains("## Evidence"));
    assert!(writer["draft_markdown"]
        .as_str()
        .unwrap()
        .contains("## Claim refs"));

    let foreman: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(chunk_dir.join("foreman-manifest.json")).unwrap())
            .unwrap();
    assert_eq!(foreman["role"], "foreman");
    assert_eq!(foreman["status"], "passed");
    assert_eq!(foreman["writer_count"], 3);
    assert_eq!(foreman["auditor_count"], 3);
    assert_eq!(foreman["acceptance"]["all_auditors_passed"], true);
    assert_eq!(foreman["acceptance"]["standardized_format"], true);
    assert_eq!(foreman["acceptance"]["traceable"], true);
    assert_eq!(foreman["acceptance"]["controls_final_write"], true);
    let generated_text = fs::read_to_string(vault.join("brain/daily/2026-06-10.md")).unwrap();
    assert!(generated_text.contains("## Kanban 精炼 Harness"));
    assert!(generated_text.contains("final Markdown is written only after foreman acceptance"));

    let validation = jianling_validate_run(&vault, &report.run_id).unwrap();
    assert!(
        validation.ok,
        "chunked run should validate: {validation:#?}"
    );

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_validate_run_rejects_tampered_kanban_auditor_card() {
    let vault = temp_vault("kanban-tamper");
    seed_many_raw_dialogues(&vault, 45);

    let report = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "default".to_string(),
            mode: JianlingRunMode::Daily,
            dry_run: false,
            scheduled: false,
            db: None,
            date: Some("2026-06-10".to_string()),
            max_source_files: 45,
        },
    )
    .unwrap();
    let chunk_dir = Path::new(report.chunk_dir.as_ref().unwrap());
    let auditor_path = chunk_dir.join("auditor-000.json");
    let mut auditor: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&auditor_path).unwrap()).unwrap();
    auditor["checks"]["traceability"] = serde_json::Value::String("failed".to_string());
    fs::write(
        &auditor_path,
        serde_json::to_string_pretty(&auditor).unwrap(),
    )
    .unwrap();

    let validation = jianling_validate_run(&vault, &report.run_id).unwrap();
    assert!(
        !validation.ok,
        "tampered auditor card must fail validate-run"
    );
    assert!(validation
        .error_codes
        .contains(&"invalid_kanban_foreman".to_string()));

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_doctor_reports_self_check_components() {
    let vault = temp_vault("doctor-self-check");
    seed_raw_dialogue(&vault);

    let doctor = jianling_doctor(&vault, "default").unwrap();
    for component in ["llm_profile", "global_run_lock", "brain_output_paths"] {
        assert!(
            doctor
                .checks
                .iter()
                .any(|check| check.component == component),
            "missing doctor component {component}: {doctor:#?}"
        );
    }

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_chat_smoke_receipts_unconfigured_llm_without_secret_leak() {
    let vault = temp_vault("chat-smoke-unconfigured");
    let _guard = ScopedEnv::clear(&[
        "ORDERK_SWORD_LLM_ANTHROPIC_API_KEY",
        "ORDERK_SWORD_LLM_MINIMAX_API_KEY",
        "ORDERK_SWORD_LLM_API_KEY",
        "ORDERK_SWORD_LLM_API_KEY_ENV",
        "ORDERK_JIANLING_LLM_ENABLED",
    ]);

    let smoke = jianling_chat_smoke(&vault, "default").unwrap();
    assert!(!smoke.ok);
    assert_eq!(smoke.status, "llm_unconfigured");
    assert_eq!(smoke.verification_mode, "live");
    assert!(Path::new(&smoke.receipt_path).is_file());
    let raw = fs::read_to_string(&smoke.receipt_path).unwrap();
    assert!(!raw.contains("sk-"));
    assert!(!raw.contains("Bearer"));

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_apply_configured_llm_without_hot_switch_does_not_call_provider() {
    let vault = temp_vault("live-llm-switch-off");
    seed_raw_dialogue(&vault);
    let server = FakeAnthropicServer::start("- should not be called\n");
    let _guard = ScopedEnv::set_with_clear(
        &[
            ("ORDERK_SWORD_LLM_API_KEY_ENV", "ORDERK_TEST_LLM_KEY"),
            ("ORDERK_TEST_LLM_KEY", "test-secret"),
            ("ORDERK_SWORD_LLM_BASE_URL", server.base_url.as_str()),
        ],
        &[
            "ORDERK_JIANLING_LLM_ENABLED",
            "ORDERK_JIANLING_LLM_ENABLED_SWITCHOFF",
        ],
    );

    let report = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "switchoff".to_string(),
            mode: JianlingRunMode::Daily,
            dry_run: false,
            scheduled: false,
            db: None,
            date: Some("2026-06-10".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();

    assert_eq!(
        report.provider_status,
        "configured_inactive_explicit_switch_off"
    );
    assert_eq!(server.request_count(), 0);
    let daily_text = fs::read_to_string(vault.join("brain/daily/2026-06-10.md")).unwrap();
    assert!(!daily_text.contains("## LLM 反思（MiniMax M3）"));

    let _ = fs::remove_dir_all(vault);
}

#[test]
fn jianling_apply_calls_configured_llm_and_writes_reflection() {
    let vault = temp_vault("live-llm-run");
    seed_raw_dialogue(&vault);
    let server = FakeAnthropicServer::start("- LLM reflection from fake MiniMax [S1]\n");
    let _guard = ScopedEnv::set(&[
        ("ORDERK_JIANLING_LLM_ENABLED_LLMMODE", "1"),
        ("ORDERK_SWORD_LLM_API_KEY_ENV", "ORDERK_TEST_LLM_KEY"),
        ("ORDERK_TEST_LLM_KEY", "test-secret"),
        ("ORDERK_SWORD_LLM_BASE_URL", server.base_url.as_str()),
    ]);

    let report = jianling_run(
        &vault,
        &JianlingRunOptions {
            profile: "llmmode".to_string(),
            mode: JianlingRunMode::Daily,
            dry_run: false,
            scheduled: false,
            db: None,
            date: Some("2026-06-10".to_string()),
            max_source_files: 20,
        },
    )
    .unwrap();

    assert_eq!(report.provider_status, "called_live");
    assert_eq!(report.success_predicate.provider, "called_live");
    let daily_text = fs::read_to_string(vault.join("brain/daily/2026-06-10.md")).unwrap();
    assert!(daily_text.contains("## LLM 反思（MiniMax M3）"));
    assert!(daily_text.contains("LLM reflection from fake MiniMax [S1]"));
    assert_eq!(server.request_count(), 1);

    let _ = fs::remove_dir_all(vault);
}

struct FakeAnthropicServer {
    base_url: String,
    count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl FakeAnthropicServer {
    fn start(text: &'static str) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count_for_thread = count.clone();
        let handle = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while std::time::Instant::now() < deadline
                && count_for_thread.load(std::sync::atomic::Ordering::SeqCst) < 3
            {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        use std::io::{Read, Write};
                        count_for_thread.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let mut buf = [0_u8; 8192];
                        let _ = stream.read(&mut buf);
                        let body = serde_json::json!({
                            "content": [{"type":"text", "text": text}]
                        })
                        .to_string();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            base_url: format!("http://{addr}"),
            count,
            handle: Some(handle),
        }
    }

    fn request_count(&self) -> usize {
        self.count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Drop for FakeAnthropicServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct ScopedEnv {
    saved: Vec<(&'static str, Option<String>)>,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl ScopedEnv {
    fn clear(names: &[&'static str]) -> Self {
        let guard = jianling_test_env_lock();
        let saved = names
            .iter()
            .map(|name| (*name, std::env::var(name).ok()))
            .collect::<Vec<_>>();
        for name in names {
            std::env::remove_var(name);
        }
        Self {
            saved,
            _guard: guard,
        }
    }

    fn set(values: &[(&'static str, &str)]) -> Self {
        Self::set_with_clear(values, &[])
    }

    fn set_with_clear(values: &[(&'static str, &str)], clear_names: &[&'static str]) -> Self {
        let guard = jianling_test_env_lock();
        let mut names = clear_names.to_vec();
        names.extend(values.iter().map(|(name, _)| *name));
        names.sort();
        names.dedup();
        let saved = names
            .iter()
            .map(|name| (*name, std::env::var(name).ok()))
            .collect::<Vec<_>>();
        for name in clear_names {
            std::env::remove_var(name);
        }
        for (name, value) in values {
            std::env::set_var(name, value);
        }
        Self {
            saved,
            _guard: guard,
        }
    }
}

impl Drop for ScopedEnv {
    fn drop(&mut self) {
        for (name, value) in self.saved.drain(..) {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
    }
}

fn jianling_test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
