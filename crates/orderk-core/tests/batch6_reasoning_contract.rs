use orderk_core::{reason_about_vault, ReasoningOptions};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_vault(prefix: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "orderk-batch6-{prefix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_vault(vault: &Path) {
    fs::create_dir_all(vault.join("wiki/concepts")).unwrap();
    fs::create_dir_all(vault.join("raw/meetings")).unwrap();
    fs::write(
        vault.join("wiki/concepts/现金流.md"),
        "# 现金流\n现金流 是资金流入与流出的节奏。它支持长期资产配置。\n",
    )
    .unwrap();
    fs::write(
        vault.join("wiki/concepts/护城河.md"),
        "# 护城河\n护城河 说明竞争优势的持续性。它和现金流共同影响商业判断。\n",
    )
    .unwrap();
    fs::write(
        vault.join("raw/meetings/cashflow.md"),
        "# 会议\n讨论现金流和护城河的关系，但还没有整理成正式结论。\n",
    )
    .unwrap();
}

fn raw_snapshot(vault: &Path) -> (String, String) {
    (
        fs::read_to_string(vault.join("wiki/concepts/现金流.md")).unwrap(),
        fs::read_to_string(vault.join("raw/meetings/cashflow.md")).unwrap(),
    )
}

#[test]
fn reasoning_no_trigger_keeps_llm_zero_and_does_not_write() {
    let vault = temp_vault("no-trigger");
    write_vault(&vault);
    let before = raw_snapshot(&vault);

    let report = reason_about_vault(
        &vault,
        ReasoningOptions {
            query: "现金流".to_string(),
            context_paths: vec!["wiki/concepts/现金流.md".to_string()],
            allow_llm: false,
            confidence_hint: Some(0.95),
        },
    )
    .unwrap();

    assert!(report.ok);
    assert!(!report.reasoning_triggered);
    assert_eq!(report.llm_calls, 0);
    assert_eq!(report.llm_invocation, "not_called_no_trigger");
    assert!(report.evidence_used.is_empty());
    assert!(report.relations_activated.is_empty());
    assert!(report.boundary.evidence_only);
    assert!(!report.boundary.direct_write_allowed);
    assert!(!report.suggested_patch.apply_allowed);
    assert_eq!(report.suggested_patch.route, "proposal_flow_only");
    assert_eq!(raw_snapshot(&vault), before);
    assert!(!vault.join(".orderk/wiki").exists());
    assert!(!vault.join(".orderk/raw").exists());
    let _ = fs::remove_dir_all(vault);
}

#[test]
fn reasoning_trigger_outputs_evidence_only_proposal_patch_without_mutating_vault() {
    let vault = temp_vault("trigger");
    write_vault(&vault);
    let before = raw_snapshot(&vault);

    let report = reason_about_vault(
        &vault,
        ReasoningOptions {
            query: "请判断现金流和护城河的架构取舍，并给出复盘结论".to_string(),
            context_paths: vec![
                "wiki/concepts/现金流.md".to_string(),
                "wiki/concepts/护城河.md".to_string(),
                "raw/meetings/cashflow.md".to_string(),
            ],
            allow_llm: true,
            confidence_hint: Some(0.41),
        },
    )
    .unwrap();

    assert!(report.ok);
    assert!(report.reasoning_triggered);
    assert!(report
        .trigger_reasons
        .contains(&"explicit_high_level_intent".to_string()));
    assert!(report
        .trigger_reasons
        .contains(&"low_confidence".to_string()));
    assert_eq!(
        report.llm_calls, 0,
        "Batch 6 deterministic path must not call LLM in tests"
    );
    assert_eq!(report.mutation_policy, "no_direct_writes");
    assert!(report.raw_unchanged);
    assert!(report.evidence_used.len() >= 2);
    assert!(!report.conclusion.trim().is_empty());
    assert!((0.0..=1.0).contains(&report.confidence));
    assert_eq!(report.boundary.suggested_patch_route, "proposals");
    assert_eq!(report.suggested_patch.status, "proposal_required");
    assert!(!report.suggested_patch.apply_allowed);
    assert!(report.suggested_patch.summary.contains("proposal"));
    assert_eq!(raw_snapshot(&vault), before);
    assert!(!vault.join(".orderk/graph").join("edges.jsonl").exists());
    let _ = fs::remove_dir_all(vault);
}

#[test]
fn reasoning_evidence_excerpts_redact_common_secret_shapes() {
    let vault = temp_vault("redaction");
    fs::create_dir_all(vault.join("wiki")).unwrap();
    let provider_token = format!("{}{}", "sk-", "secret-token-1234567890abcdef");
    let fake_password = ["super", "secret", "value"].join("-");
    let fake_key_name = ["ORDERK", "SWORD", "LLM", "API", "KEY"].join("_");
    let fake_key_value = ["llm", "secret", "value"].join("-");
    let auth_prefix = ["Authorization", "Bearer"].join(": ");
    let mut markdown = String::new();
    markdown.push_str("# ");
    markdown.push_str(&auth_prefix);
    markdown.push(' ');
    markdown.push_str(&provider_token);
    markdown.push('\n');
    markdown.push_str(&auth_prefix);
    markdown.push(' ');
    markdown.push_str(&provider_token);
    markdown.push_str(" password=");
    markdown.push_str(&fake_password);
    markdown.push(' ');
    markdown.push_str(&fake_key_name);
    markdown.push('=');
    markdown.push_str(&fake_key_value);
    markdown.push_str("\n判断策略前必须看证据。\n");
    fs::write(vault.join("wiki/ops.md"), markdown).unwrap();

    let report = reason_about_vault(
        &vault,
        ReasoningOptions {
            query: "请判断 Ops 策略风险".to_string(),
            context_paths: vec!["wiki/ops.md".to_string()],
            allow_llm: true,
            confidence_hint: Some(0.2),
        },
    )
    .unwrap();

    let serialized = serde_json::to_string(&report.evidence_used).unwrap();
    assert!(!serialized.contains(&provider_token), "{serialized}");
    assert!(!serialized.contains(&fake_password), "{serialized}");
    assert!(!serialized.contains(&fake_key_value), "{serialized}");
    assert!(!serialized.contains("Bearer sk-"), "{serialized}");
    assert!(serialized.contains("[REDACTED]"), "{serialized}");
    let _ = fs::remove_dir_all(vault);
}
#[test]
fn reasoning_rejects_unsafe_context_paths() {
    let vault = temp_vault("unsafe-context");
    write_vault(&vault);
    let err = reason_about_vault(
        &vault,
        ReasoningOptions {
            query: "判断现金流".to_string(),
            context_paths: vec!["../outside.md".to_string()],
            allow_llm: false,
            confidence_hint: None,
        },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("unsafe reasoning context path"),
        "{err:#}"
    );
    let _ = fs::remove_dir_all(vault);
}
