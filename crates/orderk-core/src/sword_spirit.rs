use crate::markdown::parse_markdown;
use crate::models::ScannedFile;
use crate::scanner::scan_vault;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_MAX_FILES: usize = 200;
const DEFAULT_MAX_PROPOSALS: usize = 100;
const DEFAULT_LLM_PROVIDER: &str = "anthropic";
const DEFAULT_LLM_MODEL: &str = "MiniMax-M3";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwordSpiritOptions {
    pub max_files: usize,
    pub max_proposals: usize,
    pub llm_provider: String,
    pub llm_model: String,
}

impl Default for SwordSpiritOptions {
    fn default() -> Self {
        Self {
            max_files: DEFAULT_MAX_FILES,
            max_proposals: DEFAULT_MAX_PROPOSALS,
            llm_provider: DEFAULT_LLM_PROVIDER.to_string(),
            llm_model: DEFAULT_LLM_MODEL.to_string(),
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
    pub proposals_path: String,
    pub audit_path: String,
    pub manifest_path: String,
    pub report_path: String,
    pub boundary: SwordSpiritBoundary,
    pub llm: SwordSpiritLlmMetadata,
    pub proposals: Vec<SwordSpiritProposal>,
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
            "files_scanned={} exceeds max_files={}; MVP considered the first sorted subset only",
            files.len(),
            options.max_files
        ));
    }
    let considered: Vec<ScannedFile> = files.iter().take(options.max_files).cloned().collect();
    let documents = read_documents(&considered)?;
    let mut proposals = generate_proposals(&documents, options.max_proposals);
    proposals.sort_by(|a, b| a.id.cmp(&b.id));

    let manifest_path = run_dir.join("input-manifest.json");
    let proposals_path = run_dir.join("proposals.jsonl");
    let audit_path = run_dir.join("audit.jsonl");
    let report_path = run_dir.join("report.md");

    let manifest = SwordSpiritManifest {
        schema_version: "orderk.sword_spirit.input_manifest.v1",
        vault: vault.to_string_lossy().to_string(),
        files: &considered,
    };
    write_json_pretty(&manifest_path, &manifest)?;
    write_jsonl(&proposals_path, &proposals)?;
    write_jsonl(
        &audit_path,
        &[SwordSpiritAuditEvent {
            schema_version: "orderk.sword_spirit.audit_event.v1",
            event: "proposal_run_created",
            run_id: &run_id,
            proposal_count: proposals.len(),
            created_at: Utc::now().to_rfc3339(),
            note: "MVP generated sidecar proposals only; no Markdown source files were modified.",
        }],
    )?;
    write_report(&report_path, &run_id, &vault, &proposals, options)?;

    Ok(SwordSpiritRunResponse {
        ok: true,
        schema_version: "orderk.sword_spirit.run.v1".to_string(),
        mode: "proposal".to_string(),
        vault: vault.to_string_lossy().to_string(),
        run_id,
        sidecar_root: sidecar_root.to_string_lossy().to_string(),
        run_dir: run_dir.to_string_lossy().to_string(),
        files_scanned: files.len(),
        files_considered: considered.len(),
        proposal_count: proposals.len(),
        proposals_path: proposals_path.to_string_lossy().to_string(),
        audit_path: audit_path.to_string_lossy().to_string(),
        manifest_path: manifest_path.to_string_lossy().to_string(),
        report_path: report_path.to_string_lossy().to_string(),
        boundary: sword_spirit_boundary(),
        llm: SwordSpiritLlmMetadata {
            provider: options.llm_provider.clone(),
            model: options.llm_model.clone(),
            invocation: "not_called_mvp".to_string(),
            note: "MVP records the Hindsight-aligned model choice but uses deterministic proposal heuristics; no API key is read or logged.".to_string(),
        },
        proposals,
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
        });
    }
    Ok(docs)
}

fn generate_proposals(
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
        rationale: "A visible H1 title would improve human Markdown-base navigation and orderk result labels; MVP only proposes it.".to_string(),
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
            "{}-{}-{}-{}",
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
    options: &SwordSpiritOptions,
) -> Result<()> {
    let mut body = String::new();
    body.push_str("# orderk Sword Spirit MVP Report\n\n");
    body.push_str(&format!("- run_id: `{run_id}`\n"));
    body.push_str(&format!("- vault: `{}`\n", vault.display()));
    body.push_str("- mode: `proposal`\n");
    body.push_str(&format!(
        "- llm: `{}` / `{}` (not called in MVP)\n",
        options.llm_provider, options.llm_model
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
            ".orderk/sword_spirit/runs/<run_id>/proposals.jsonl".to_string(),
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

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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
        assert_eq!(response.files_scanned, 2, ".orderk sidecar must be ignored");
        assert_eq!(fs::read_to_string(vault.join("alpha.md")).unwrap(), before);
        assert!(Path::new(&response.proposals_path).exists());
        assert!(Path::new(&response.audit_path).exists());
        assert!(Path::new(&response.report_path).exists());
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
}
