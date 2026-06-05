use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use orderk_core::digest::{digest_vault, DigestOptions};
use orderk_core::graph::{
    bounded_graph_boost, explain_graph, rebuild_graph, GraphBuildOptions, GraphEdgeRelation,
    GraphEdgeState,
};

fn temp_vault(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "orderk-batch5-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_sidecar_proposal(vault: &Path, id: &str, relation: &str, source: &str, target: &str) {
    let run = vault
        .join(".orderk")
        .join("sword_spirit")
        .join("runs")
        .join("run-1");
    fs::create_dir_all(&run).unwrap();
    let body = serde_json::json!({
        "schema_version": "orderk.sword_spirit.proposal.v1",
        "id": id,
        "proposal_type": "semantic_neighbor",
        "relation": relation,
        "source_path": source,
        "target_path": target,
        "confidence": 0.82,
        "risk": "review",
        "auto_apply": false,
        "human_review_required": true,
        "evidence": [{"path": target, "kind": "test", "value": "fixture evidence"}],
        "rationale": "fixture proposal"
    });
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(run.join("proposals.jsonl"))
        .unwrap();
    writeln!(file, "{body}").unwrap();
}

#[test]
fn graph_rebuild_accepts_only_prd_relations_and_explain_is_json_ready() {
    let vault = temp_vault("graph-relations");
    fs::write(vault.join("alpha.md"), "# Alpha\nSee [[Bravo]].\n").unwrap();
    fs::write(vault.join("bravo.md"), "# Bravo\n").unwrap();
    write_sidecar_proposal(&vault, "p1", "supports", "alpha.md", "bravo.md");
    write_sidecar_proposal(&vault, "p2", "made_up_relation", "alpha.md", "bravo.md");

    let graph = rebuild_graph(&vault, GraphBuildOptions { apply: false }).unwrap();

    assert_eq!(
        GraphEdgeRelation::allowed_values(),
        [
            "supports",
            "refines",
            "contradicts",
            "replaces",
            "depends_on",
            "part_of"
        ]
    );
    assert!(graph.edges.iter().any(|edge| {
        edge.source_path == "alpha.md"
            && edge.target_path == "bravo.md"
            && edge.relation == GraphEdgeRelation::Supports
    }));
    assert!(graph
        .rejected_edges
        .iter()
        .any(|edge| edge.proposal_id.as_deref() == Some("p2")));

    let explain = explain_graph(&vault, "alpha.md").unwrap();
    assert_eq!(explain.query, "alpha.md");
    assert!(explain
        .outgoing
        .iter()
        .any(|edge| edge.target_path == "bravo.md"));
    assert!(serde_json::to_value(&explain)
        .unwrap()
        .get("outgoing")
        .is_some());
    let _ = fs::remove_dir_all(vault);
}

#[test]
fn graph_rejects_non_prd_sidecar_relation_unsafe_paths_and_missing_evidence_overlap() {
    let vault = temp_vault("graph-rejects-sidecar");
    fs::write(vault.join("alpha.md"), "# Alpha\n").unwrap();
    fs::write(vault.join("bravo.md"), "# Bravo\n").unwrap();
    write_sidecar_proposal(&vault, "p_wikilink", "wikilink", "alpha.md", "bravo.md");
    write_sidecar_proposal(&vault, "p_escape", "supports", "../outside.md", "bravo.md");

    let run = vault
        .join(".orderk")
        .join("sword_spirit")
        .join("runs")
        .join("run-1");
    let mismatch = serde_json::json!({
        "schema_version": "orderk.sword_spirit.proposal.v1",
        "id": "p_mismatch",
        "proposal_type": "semantic_neighbor",
        "relation": "supports",
        "source_path": "alpha.md",
        "target_path": "bravo.md",
        "confidence": 0.82,
        "risk": "review",
        "auto_apply": false,
        "human_review_required": true,
        "evidence": [{"path": "alpha.md", "kind": "test", "value": "target missing from evidence"}],
        "rationale": "fixture proposal"
    });
    let mut file = OpenOptions::new()
        .append(true)
        .open(run.join("proposals.jsonl"))
        .unwrap();
    writeln!(file, "{mismatch}").unwrap();

    let graph = rebuild_graph(&vault, GraphBuildOptions { apply: false }).unwrap();

    assert!(
        graph
            .rejected_edges
            .iter()
            .any(|edge| edge.proposal_id.as_deref() == Some("p_wikilink")
                && edge.reason.contains("relation_not_in_prd_allowlist")),
        "wikilink sidecar relation must not be silently normalized into supports: {:#?}",
        graph.rejected_edges
    );
    assert!(
        graph
            .rejected_edges
            .iter()
            .any(|edge| edge.proposal_id.as_deref() == Some("p_escape")
                && edge.reason.contains("unsafe_or_missing_source_path")),
        "unsafe source path must be rejected: {:#?}",
        graph.rejected_edges
    );
    assert!(
        graph
            .rejected_edges
            .iter()
            .any(|edge| edge.proposal_id.as_deref() == Some("p_mismatch")
                && edge.reason.contains("target_path_not_in_evidence_set")),
        "target must overlap evidence: {:#?}",
        graph.rejected_edges
    );
    let _ = fs::remove_dir_all(vault);
}

#[cfg(unix)]
#[test]
fn graph_and_digest_apply_reject_symlinked_sidecar_paths() {
    use std::os::unix::fs::symlink;

    let vault = temp_vault("sidecar-symlink");
    fs::write(vault.join("alpha.md"), "# Alpha\n").unwrap();
    let outside = temp_vault("outside-sidecar-target");
    fs::create_dir_all(vault.join(".orderk")).unwrap();
    symlink(&outside, vault.join(".orderk/graph")).unwrap();

    let graph_err = rebuild_graph(&vault, GraphBuildOptions { apply: true }).unwrap_err();
    assert!(
        graph_err.to_string().contains("symlink"),
        "graph apply must reject symlinked sidecar dir: {graph_err:#}"
    );

    fs::remove_file(vault.join(".orderk/graph")).unwrap();
    symlink(&outside, vault.join(".orderk/digest")).unwrap();
    let digest_err = digest_vault(
        &vault,
        DigestOptions {
            profile: "default".to_string(),
            apply: true,
            resume: false,
        },
    )
    .unwrap_err();
    assert!(
        digest_err.to_string().contains("symlink"),
        "digest apply must reject symlinked sidecar dir: {digest_err:#}"
    );

    let _ = fs::remove_dir_all(vault);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn graph_rebuild_applies_audit_states_and_persists_store_without_raw_writes() {
    let vault = temp_vault("graph-audit");
    fs::write(vault.join("alpha.md"), "# Alpha\n").unwrap();
    fs::write(vault.join("bravo.md"), "# Bravo\n").unwrap();
    write_sidecar_proposal(&vault, "p1", "refines", "alpha.md", "bravo.md");
    let audit_root = vault.join(".orderk").join("proposals");
    fs::create_dir_all(&audit_root).unwrap();
    fs::write(
        audit_root.join("audit.jsonl"),
        serde_json::json!({
            "schema_version":"orderk.proposals.audit_event.v1",
            "event":"approved",
            "proposal_id":"p1",
            "run_id":"run-1",
            "status":"active",
            "dry_run":false,
            "apply":true,
            "reason":null,
            "created_at":"2026-06-05T00:00:00Z"
        })
        .to_string()
            + "\n",
    )
    .unwrap();
    let before = fs::read_to_string(vault.join("alpha.md")).unwrap();

    let graph = rebuild_graph(&vault, GraphBuildOptions { apply: true }).unwrap();

    assert!(graph
        .edges
        .iter()
        .any(|edge| edge.proposal_id.as_deref() == Some("p1")
            && edge.state == GraphEdgeState::Active));
    assert!(vault.join(".orderk/graph/edges.jsonl").is_file());
    assert_eq!(fs::read_to_string(vault.join("alpha.md")).unwrap(), before);
    let _ = fs::remove_dir_all(vault);
}

#[test]
fn graph_boost_is_bounded_and_cannot_demote_base_top() {
    let boost = bounded_graph_boost(0.91, 100);

    assert!(
        boost <= 0.03,
        "graph boost must be observational and bounded: {boost}"
    );
    assert_eq!(
        bounded_graph_boost(0.99, 100),
        0.0,
        "high-confidence base top must not be boosted over itself"
    );
}

#[test]
fn digest_dry_run_detects_changes_apply_records_state_and_lock_is_fail_closed() {
    let vault = temp_vault("digest");
    fs::write(vault.join("alpha.md"), "# Alpha\nfirst\n").unwrap();

    let dry = digest_vault(
        &vault,
        DigestOptions {
            profile: "default".to_string(),
            apply: false,
            resume: false,
        },
    )
    .unwrap();
    assert_eq!(dry.changed_paths, vec!["alpha.md"]);
    assert!(!vault.join(".orderk/digest/state.json").exists());

    let applied = digest_vault(
        &vault,
        DigestOptions {
            profile: "default".to_string(),
            apply: true,
            resume: false,
        },
    )
    .unwrap();
    assert!(applied.state_written);
    assert!(vault.join(".orderk/digest/state.json").is_file());

    fs::write(vault.join("alpha.md"), "# Alpha\nsecond\n").unwrap();
    let changed = digest_vault(
        &vault,
        DigestOptions {
            profile: "default".to_string(),
            apply: false,
            resume: false,
        },
    )
    .unwrap();
    assert_eq!(changed.changed_paths, vec!["alpha.md"]);

    fs::write(vault.join(".orderk/digest/digest.lock"), "running\n").unwrap();
    let locked = digest_vault(
        &vault,
        DigestOptions {
            profile: "default".to_string(),
            apply: true,
            resume: false,
        },
    )
    .unwrap_err();
    assert!(
        locked.to_string().contains("digest lock exists"),
        "{locked:#}"
    );

    let resumed = digest_vault(
        &vault,
        DigestOptions {
            profile: "default".to_string(),
            apply: true,
            resume: true,
        },
    )
    .unwrap();
    assert!(resumed.state_written);
    assert!(!vault.join(".orderk/digest/digest.lock").exists());
    let _ = fs::remove_dir_all(vault);
}
