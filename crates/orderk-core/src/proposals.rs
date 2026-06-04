use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::sword_spirit::SwordSpiritProposal;

const PROPOSAL_AUDIT_SCHEMA: &str = "orderk.proposals.audit_event.v1";
const PROPOSAL_ALLOWLIST_SCHEMA: &str = "orderk.proposals.allowlist.v1";
const MAX_BACKLOG: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GovernedProposal {
    pub schema_version: String,
    pub id: String,
    pub run_id: String,
    pub relation: Option<String>,
    pub source_path: String,
    pub target_path: Option<String>,
    pub confidence: f32,
    pub risk: String,
    pub status: String,
    pub auto_apply: bool,
    pub human_review_required: bool,
    pub evidence_paths: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProposalBacklog {
    pub schema_version: String,
    pub vault: String,
    pub proposals_root: String,
    pub audit_path: String,
    pub total_sidecar_proposals: usize,
    pub duplicates_deduped: usize,
    pub backlog_cap: usize,
    pub capped: bool,
    pub proposals: Vec<GovernedProposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProposalShow {
    pub schema_version: String,
    pub proposal: GovernedProposal,
    pub diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProposalDecisionResult {
    pub schema_version: String,
    pub proposal_id: String,
    pub run_id: String,
    pub action: String,
    pub status: String,
    pub dry_run: bool,
    pub apply: bool,
    pub audit_path: String,
    pub audit_written: bool,
    pub allowlist_required: bool,
    pub allowlist_path: String,
    pub evidence_gate: String,
    pub diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProposalAuditEvent {
    schema_version: String,
    event: String,
    proposal_id: String,
    run_id: String,
    status: String,
    dry_run: bool,
    apply: bool,
    reason: Option<String>,
    created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ProposalAllowlist {
    #[allow(dead_code)]
    schema_version: Option<String>,
    #[serde(default)]
    proposal_ids: Vec<String>,
}

pub fn proposal_paths(vault: &Path) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let root = vault.join(".orderk").join("proposals");
    let audit = root.join("audit.jsonl");
    let allowlist = root.join("allowlist.json");
    Ok((root, audit, allowlist))
}

pub fn list_proposals(vault: &Path) -> Result<ProposalBacklog> {
    list_proposals_with_cap(vault, MAX_BACKLOG)
}

pub fn list_proposals_with_cap(vault: &Path, cap: usize) -> Result<ProposalBacklog> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let (root, audit_path, _allowlist_path) = proposal_paths(&vault)?;
    let statuses = audit_statuses(&audit_path)?;
    let mut seen = HashSet::new();
    let mut proposals = Vec::new();
    let mut total = 0usize;
    let runs_dir = vault.join(".orderk").join("sword_spirit").join("runs");
    let mut runs = Vec::new();
    if runs_dir.is_dir() {
        for entry in fs::read_dir(&runs_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() && entry.path().join("proposals.jsonl").is_file() {
                runs.push(entry.path());
            }
        }
    }
    runs.sort();
    runs.reverse();
    for run_dir in runs {
        let run_id = run_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("invalid run dir: {}", run_dir.display()))?
            .to_string();
        let proposal_path = run_dir.join("proposals.jsonl");
        for proposal in read_sidecar_proposals(&proposal_path)? {
            total += 1;
            if !seen.insert(proposal.id.clone()) {
                continue;
            }
            if proposals.len() >= cap {
                continue;
            }
            let status_override = statuses.get(&proposal.id);
            proposals.push(governed_proposal(proposal, &run_id, status_override));
        }
    }
    let duplicates_deduped = total.saturating_sub(seen.len());
    let capped = seen.len() > cap;
    Ok(ProposalBacklog {
        schema_version: "orderk.proposals.backlog.v1".to_string(),
        vault: vault.to_string_lossy().to_string(),
        proposals_root: root.to_string_lossy().to_string(),
        audit_path: audit_path.to_string_lossy().to_string(),
        total_sidecar_proposals: total,
        duplicates_deduped,
        backlog_cap: cap,
        capped,
        proposals,
    })
}

pub fn show_proposal(vault: &Path, id: &str) -> Result<ProposalShow> {
    let proposal = find_proposal(vault, id)?;
    Ok(ProposalShow {
        schema_version: "orderk.proposals.show.v1".to_string(),
        diff: proposal_diff(&proposal),
        proposal,
    })
}

pub fn approve_proposal(
    vault: &Path,
    id: &str,
    dry_run: bool,
    apply: bool,
) -> Result<ProposalDecisionResult> {
    if dry_run == apply {
        return Err(anyhow!(
            "approve requires exactly one of --dry-run or --apply"
        ));
    }
    let proposal = find_proposal(vault, id)?;
    ensure_evidence_gate(&proposal)?;
    let (root, audit_path, allowlist_path) = proposal_paths(vault)?;
    if apply {
        ensure_allowlisted(&allowlist_path, id)?;
    }
    let diff = proposal_diff(&proposal);
    let audit_written = if apply {
        append_audit(
            vault,
            &root,
            &audit_path,
            &ProposalAuditEvent {
                schema_version: PROPOSAL_AUDIT_SCHEMA.to_string(),
                event: "approved".to_string(),
                proposal_id: id.to_string(),
                run_id: proposal.run_id.clone(),
                status: "active".to_string(),
                dry_run: false,
                apply: true,
                reason: None,
                created_at: Utc::now().to_rfc3339(),
            },
        )?;
        true
    } else {
        false
    };
    Ok(ProposalDecisionResult {
        schema_version: "orderk.proposals.decision.v1".to_string(),
        proposal_id: id.to_string(),
        run_id: proposal.run_id,
        action: "approve".to_string(),
        status: if apply { "active" } else { "dry_run" }.to_string(),
        dry_run,
        apply,
        audit_path: audit_path.to_string_lossy().to_string(),
        audit_written,
        allowlist_required: apply,
        allowlist_path: allowlist_path.to_string_lossy().to_string(),
        evidence_gate: "target_path_in_evidence_set".to_string(),
        diff,
    })
}

pub fn reject_proposal(vault: &Path, id: &str, reason: &str) -> Result<ProposalDecisionResult> {
    if reason.trim().is_empty() {
        return Err(anyhow!("reject requires a non-empty --reason"));
    }
    let proposal = find_proposal(vault, id)?;
    let (root, audit_path, allowlist_path) = proposal_paths(vault)?;
    append_audit(
        vault,
        &root,
        &audit_path,
        &ProposalAuditEvent {
            schema_version: PROPOSAL_AUDIT_SCHEMA.to_string(),
            event: "rejected".to_string(),
            proposal_id: id.to_string(),
            run_id: proposal.run_id.clone(),
            status: "rejected".to_string(),
            dry_run: false,
            apply: false,
            reason: Some(reason.trim().to_string()),
            created_at: Utc::now().to_rfc3339(),
        },
    )?;
    let diff = proposal_diff(&proposal);
    Ok(ProposalDecisionResult {
        schema_version: "orderk.proposals.decision.v1".to_string(),
        proposal_id: id.to_string(),
        run_id: proposal.run_id,
        action: "reject".to_string(),
        status: "rejected".to_string(),
        dry_run: false,
        apply: false,
        audit_path: audit_path.to_string_lossy().to_string(),
        audit_written: true,
        allowlist_required: false,
        allowlist_path: allowlist_path.to_string_lossy().to_string(),
        evidence_gate: "not_required_for_reject".to_string(),
        diff,
    })
}

pub fn proposal_diff(proposal: &GovernedProposal) -> String {
    let target = proposal
        .target_path
        .as_deref()
        .unwrap_or("<missing-target>");
    format!(
        "--- a/{source}\n+++ b/{target}\n@@ semantic proposal {id}\nrelation: {relation}\nconfidence: {confidence:.3}\nstatus: {status}\nrationale: {rationale}\n",
        source = proposal.source_path,
        target = target,
        id = proposal.id,
        relation = proposal.relation.as_deref().unwrap_or("unknown"),
        confidence = proposal.confidence,
        status = proposal.status,
        rationale = proposal.rationale.replace('\n', " "),
    )
}

fn governed_proposal(
    proposal: SwordSpiritProposal,
    run_id: &str,
    status_override: Option<&String>,
) -> GovernedProposal {
    let evidence_paths = proposal
        .evidence
        .iter()
        .map(|evidence| evidence.path.clone())
        .filter(|path| !path.trim().is_empty())
        .collect::<Vec<_>>();
    GovernedProposal {
        schema_version: "orderk.proposals.item.v1".to_string(),
        id: proposal.id,
        run_id: run_id.to_string(),
        relation: proposal.relation,
        source_path: proposal.source_path,
        target_path: proposal.target_path,
        confidence: proposal.confidence,
        risk: proposal.risk,
        status: status_override
            .cloned()
            .unwrap_or_else(|| "proposal".to_string()),
        auto_apply: proposal.auto_apply,
        human_review_required: proposal.human_review_required,
        evidence_paths,
        rationale: proposal.rationale,
    }
}

fn find_proposal(vault: &Path, id: &str) -> Result<GovernedProposal> {
    let backlog = list_proposals(vault)?;
    backlog
        .proposals
        .into_iter()
        .find(|proposal| proposal.id == id)
        .ok_or_else(|| anyhow!("proposal not found: {id}"))
}

fn read_sidecar_proposals(path: &Path) -> Result<Vec<SwordSpiritProposal>> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut out = Vec::new();
    for (line_no, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        out.push(
            serde_json::from_str(trimmed)
                .with_context(|| format!("parse {} line {}", path.display(), line_no + 1))?,
        );
    }
    Ok(out)
}

fn audit_statuses(path: &Path) -> Result<HashMap<String, String>> {
    let mut statuses = HashMap::new();
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Ok(statuses);
    };
    if meta.file_type().is_symlink() {
        return Err(anyhow!(
            "refusing to read audit through symlink: {}",
            path.display()
        ));
    }
    if !meta.is_file() {
        return Ok(statuses);
    }
    let raw = fs::read_to_string(path)?;
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let event: ProposalAuditEvent = serde_json::from_str(line)?;
        statuses.insert(event.proposal_id, event.status);
    }
    Ok(statuses)
}

fn append_audit(vault: &Path, root: &Path, path: &Path, event: &ProposalAuditEvent) -> Result<()> {
    prepare_audit_path(vault, root, path)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, event)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn prepare_audit_path(vault: &Path, root: &Path, audit_path: &Path) -> Result<()> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let orderk_dir = vault.join(".orderk");
    for dir in [&orderk_dir, root] {
        if let Ok(meta) = fs::symlink_metadata(dir) {
            if meta.file_type().is_symlink() {
                return Err(anyhow!(
                    "refusing to use symlinked proposals directory: {}",
                    dir.display()
                ));
            }
            if !meta.is_dir() {
                return Err(anyhow!(
                    "proposal path is not a directory: {}",
                    dir.display()
                ));
            }
        }
    }
    fs::create_dir_all(root)?;
    let canonical_root = root.canonicalize()?;
    if !canonical_root.starts_with(&vault) {
        return Err(anyhow!(
            "proposal audit directory escapes vault: {}",
            root.display()
        ));
    }
    if let Ok(meta) = fs::symlink_metadata(audit_path) {
        if meta.file_type().is_symlink() {
            return Err(anyhow!(
                "refusing to append audit through symlink: {}",
                audit_path.display()
            ));
        }
        if !meta.is_file() {
            return Err(anyhow!(
                "proposal audit path is not a file: {}",
                audit_path.display()
            ));
        }
    }
    Ok(())
}

fn ensure_allowlisted(path: &Path, id: &str) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(anyhow!(
                "refusing to read allowlist through symlink: {}",
                path.display()
            ));
        }
    }
    let raw = fs::read_to_string(path).with_context(|| {
        format!(
            "proposal apply is fail-closed; create local allowlist {} with schema_version {PROPOSAL_ALLOWLIST_SCHEMA} and proposal_ids containing {id}",
            path.display()
        )
    })?;
    let allowlist: ProposalAllowlist = serde_json::from_str(&raw)?;
    if allowlist.proposal_ids.iter().any(|allowed| allowed == id) {
        Ok(())
    } else {
        Err(anyhow!(
            "proposal apply blocked by local allowlist: {id} is not listed in {}",
            path.display()
        ))
    }
}

fn ensure_evidence_gate(proposal: &GovernedProposal) -> Result<()> {
    let _source = normalize_vault_relative_path("source_path", &proposal.source_path)?;
    let target = normalize_vault_relative_path(
        "target_path",
        proposal
            .target_path
            .as_deref()
            .ok_or_else(|| anyhow!("proposal target_path is required for approval"))?,
    )?;
    let mut evidence_paths = HashSet::new();
    for evidence_path in &proposal.evidence_paths {
        evidence_paths.insert(normalize_vault_relative_path(
            "evidence_path",
            evidence_path,
        )?);
    }
    if evidence_paths.contains(&target) {
        Ok(())
    } else {
        Err(anyhow!(
            "proposal evidence gate failed: target_path {target} is outside candidate evidence set"
        ))
    }
}

fn normalize_vault_relative_path(label: &str, raw: &str) -> Result<String> {
    let raw = raw.trim().replace('\\', "/");
    if raw.is_empty() {
        return Err(anyhow!("unsafe proposal {label}: empty path"));
    }
    let path = Path::new(&raw);
    if path.is_absolute() {
        return Err(anyhow!("unsafe proposal {label}: absolute path {raw}"));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(anyhow!("unsafe proposal {label}: path escapes vault {raw}"));
            }
        }
    }
    if parts.is_empty() {
        return Err(anyhow!("unsafe proposal {label}: empty path"));
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sword_spirit::SwordSpiritEvidence;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_vault(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "orderk-proposals-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn proposal(id: &str, target: &str) -> SwordSpiritProposal {
        SwordSpiritProposal {
            schema_version: "orderk.sword_spirit.proposal.v1".to_string(),
            id: id.to_string(),
            proposal_type: "semantic_neighbor".to_string(),
            relation: Some("supports".to_string()),
            source_path: "source.md".to_string(),
            target_path: Some(target.to_string()),
            confidence: 0.9,
            risk: "review".to_string(),
            auto_apply: false,
            human_review_required: true,
            evidence: vec![SwordSpiritEvidence {
                path: target.to_string(),
                kind: "test".to_string(),
                value: "target evidence".to_string(),
            }],
            rationale: "test rationale".to_string(),
        }
    }

    fn write_run(vault: &Path, run_id: &str, proposals: &[SwordSpiritProposal]) -> PathBuf {
        let run = vault
            .join(".orderk")
            .join("sword_spirit")
            .join("runs")
            .join(run_id);
        fs::create_dir_all(&run).unwrap();
        let body = proposals
            .iter()
            .map(|proposal| serde_json::to_string(proposal).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(run.join("proposals.jsonl"), format!("{body}\n")).unwrap();
        run
    }

    #[test]
    fn proposals_list_dedupes_and_caps_latest_sidecar_backlog() {
        let vault = temp_vault("list");
        write_run(&vault, "run-1", &[proposal("p1", "target.md")]);
        write_run(
            &vault,
            "run-2",
            &[proposal("p1", "target.md"), proposal("p2", "other.md")],
        );

        let backlog = list_proposals_with_cap(&vault, 1).unwrap();

        assert_eq!(backlog.total_sidecar_proposals, 3);
        assert_eq!(backlog.duplicates_deduped, 1);
        assert!(backlog.capped);
        assert_eq!(backlog.proposals.len(), 1);
        assert_eq!(backlog.proposals[0].run_id, "run-2");
        let _ = fs::remove_dir_all(vault);
    }

    #[test]
    fn proposals_approve_dry_run_is_safe_and_apply_is_fail_closed_without_allowlist() {
        let vault = temp_vault("approve");
        write_run(&vault, "run-1", &[proposal("p1", "target.md")]);

        let dry = approve_proposal(&vault, "p1", true, false).unwrap();
        assert!(!dry.audit_written);
        assert!(!Path::new(&dry.audit_path).exists());

        let err = approve_proposal(&vault, "p1", false, true).unwrap_err();
        assert!(err.to_string().contains("fail-closed"), "{err:#}");
        let _ = fs::remove_dir_all(vault);
    }

    #[test]
    fn proposals_apply_requires_allowlist_and_appends_audit() {
        let vault = temp_vault("apply");
        write_run(&vault, "run-1", &[proposal("p1", "target.md")]);
        let (root, audit, allowlist) = proposal_paths(&vault).unwrap();
        fs::create_dir_all(root).unwrap();
        fs::write(
            allowlist,
            r#"{"schema_version":"orderk.proposals.allowlist.v1","proposal_ids":["p1"]}"#,
        )
        .unwrap();

        let result = approve_proposal(&vault, "p1", false, true).unwrap();

        assert!(result.audit_written);
        let audit_text = fs::read_to_string(audit).unwrap();
        assert!(audit_text.contains("\"approved\""));
        assert!(audit_text.contains("\"active\""));
        let _ = fs::remove_dir_all(vault);
    }

    #[test]
    fn proposals_evidence_gate_blocks_target_outside_evidence_set() {
        let vault = temp_vault("evidence");
        let mut item = proposal("p1", "target.md");
        item.evidence[0].path = "unrelated.md".to_string();
        write_run(&vault, "run-1", &[item]);
        let err = approve_proposal(&vault, "p1", true, false).unwrap_err();
        assert!(err.to_string().contains("outside candidate evidence set"));
        let _ = fs::remove_dir_all(vault);
    }

    #[test]
    fn proposals_approval_rejects_unsafe_paths_even_when_allowlisted() {
        let vault = temp_vault("unsafe-path");
        let mut item = proposal("p1", "../outside.md");
        item.evidence[0].path = "../outside.md".to_string();
        write_run(&vault, "run-1", &[item]);
        let (root, _audit, allowlist) = proposal_paths(&vault).unwrap();
        fs::create_dir_all(root).unwrap();
        fs::write(
            allowlist,
            r#"{"schema_version":"orderk.proposals.allowlist.v1","proposal_ids":["p1"]}"#,
        )
        .unwrap();

        let err = approve_proposal(&vault, "p1", false, true).unwrap_err();

        assert!(
            err.to_string().contains("unsafe proposal target_path"),
            "{err:#}"
        );
        let _ = fs::remove_dir_all(vault);
    }

    #[cfg(unix)]
    #[test]
    fn proposals_apply_rejects_symlinked_audit_file() {
        use std::os::unix::fs::symlink;

        let vault = temp_vault("audit-symlink");
        write_run(&vault, "run-1", &[proposal("p1", "target.md")]);
        let (root, audit, allowlist) = proposal_paths(&vault).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            allowlist,
            r#"{"schema_version":"orderk.proposals.allowlist.v1","proposal_ids":["p1"]}"#,
        )
        .unwrap();
        let outside = vault.parent().unwrap().join("outside-audit-target.txt");
        fs::write(&outside, "outside\n").unwrap();
        symlink(&outside, &audit).unwrap();

        let err = approve_proposal(&vault, "p1", false, true).unwrap_err();

        assert!(err.to_string().contains("audit through symlink"), "{err:#}");
        assert_eq!(fs::read_to_string(outside).unwrap(), "outside\n");
        let _ = fs::remove_dir_all(vault);
    }

    #[test]
    fn proposals_reject_appends_audit_with_reason() {
        let vault = temp_vault("reject");
        write_run(&vault, "run-1", &[proposal("p1", "target.md")]);

        let result = reject_proposal(&vault, "p1", "duplicate").unwrap();

        assert!(result.audit_written);
        assert_eq!(result.status, "rejected");
        let audit = fs::read_to_string(result.audit_path).unwrap();
        assert!(audit.contains("duplicate"));
        let _ = fs::remove_dir_all(vault);
    }
}
