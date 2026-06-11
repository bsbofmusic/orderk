use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use orderk_core::{
    jianling_chat_smoke, jianling_doctor, jianling_enable, jianling_run, jianling_status,
    jianling_validate_file, jianling_validate_run, JianlingEnableOptions, JianlingRunMode,
    JianlingRunOptions, JianlingValidateFileOptions,
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
    assert_eq!(report.chunking_status, "kanban_foreman_summary_written");
    assert_eq!(report.chunk_count, 1);
    let chunk_dir = Path::new(report.chunk_dir.as_ref().unwrap());
    assert!(chunk_dir.join("writer-000.json").is_file());
    assert!(chunk_dir.join("auditor-000.json").is_file());
    assert!(chunk_dir.join("foreman-manifest.json").is_file());
    assert!(Path::new(report.foreman_summary_path.as_ref().unwrap()).is_file());
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

fn seed_many_raw_dialogues(vault: &Path, count: usize) {
    let raw = vault.join("raw/transcripts/hermes-sessions/2026/06/large");
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
        listener.set_nonblocking(false).unwrap();
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count_for_thread = count.clone();
        let handle = std::thread::spawn(move || {
            for _ in 0..3 {
                if let Ok((mut stream, _)) = listener.accept() {
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
                    break;
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
        let guard = jianling_test_env_lock();
        let saved = values
            .iter()
            .map(|(name, _)| (*name, std::env::var(name).ok()))
            .collect::<Vec<_>>();
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
