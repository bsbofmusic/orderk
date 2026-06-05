use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Component, Path};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::markdown::parse_markdown;
use crate::scanner::scan_vault;
use crate::sword_spirit::SwordSpiritProposal;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgeRelation {
    Supports,
    Refines,
    Contradicts,
    Replaces,
    DependsOn,
    PartOf,
}

impl GraphEdgeRelation {
    pub fn allowed_values() -> [&'static str; 6] {
        [
            "supports",
            "refines",
            "contradicts",
            "replaces",
            "depends_on",
            "part_of",
        ]
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Supports => "supports",
            Self::Refines => "refines",
            Self::Contradicts => "contradicts",
            Self::Replaces => "replaces",
            Self::DependsOn => "depends_on",
            Self::PartOf => "part_of",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "supports" => Some(Self::Supports),
            "refines" => Some(Self::Refines),
            "contradicts" => Some(Self::Contradicts),
            "replaces" => Some(Self::Replaces),
            "depends_on" => Some(Self::DependsOn),
            "part_of" => Some(Self::PartOf),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgeState {
    Proposal,
    Active,
    Rejected,
    Superseded,
    Conflict,
}

impl GraphEdgeState {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "proposal" => Some(Self::Proposal),
            "active" => Some(Self::Active),
            "rejected" => Some(Self::Rejected),
            "superseded" => Some(Self::Superseded),
            "conflict" => Some(Self::Conflict),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEdge {
    pub schema_version: String,
    pub id: String,
    pub source_path: String,
    pub target_path: String,
    pub relation: GraphEdgeRelation,
    pub state: GraphEdgeState,
    pub confidence: f32,
    pub source: String,
    pub proposal_id: Option<String>,
    pub evidence_paths: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RejectedGraphEdge {
    pub schema_version: String,
    pub proposal_id: Option<String>,
    pub source_path: Option<String>,
    pub target_path: Option<String>,
    pub raw_relation: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphStore {
    pub schema_version: String,
    pub vault: String,
    pub edge_count: usize,
    pub rejected_count: usize,
    pub relation_types: Vec<String>,
    pub state_types: Vec<String>,
    pub applied: bool,
    pub store_path: String,
    pub edges: Vec<GraphEdge>,
    pub rejected_edges: Vec<RejectedGraphEdge>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphBuildOptions {
    pub apply: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphExplain {
    pub schema_version: String,
    pub query: String,
    pub matched_edge: Option<GraphEdge>,
    pub outgoing: Vec<GraphEdge>,
    pub incoming: Vec<GraphEdge>,
    pub rejected: Vec<RejectedGraphEdge>,
    pub graph_store_path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ProposalAuditEvent {
    proposal_id: String,
    #[serde(default)]
    status: Option<String>,
}

pub fn rebuild_graph(vault: &Path, options: GraphBuildOptions) -> Result<GraphStore> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let mut edges = Vec::new();
    let mut rejected_edges = Vec::new();
    let markdown_edges = markdown_edges(&vault)?;
    edges.extend(markdown_edges);
    let audit_statuses = proposal_audit_statuses(&vault)?;
    read_sidecar_edges(&vault, &audit_statuses, &mut edges, &mut rejected_edges)?;
    mark_conflicts(&mut edges);
    edges.sort_by(|a, b| {
        (&a.source_path, &a.target_path, a.relation, a.state, &a.id).cmp(&(
            &b.source_path,
            &b.target_path,
            b.relation,
            b.state,
            &b.id,
        ))
    });
    rejected_edges.sort_by(|a, b| a.proposal_id.cmp(&b.proposal_id));
    let store_path = vault.join(".orderk").join("graph").join("edges.jsonl");
    if options.apply {
        write_graph_store(&vault, &store_path, &edges, &rejected_edges)?;
    }
    Ok(GraphStore {
        schema_version: "orderk.graph.store.v1".to_string(),
        vault: vault.to_string_lossy().to_string(),
        edge_count: edges.len(),
        rejected_count: rejected_edges.len(),
        relation_types: GraphEdgeRelation::allowed_values()
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        state_types: ["proposal", "active", "rejected", "superseded", "conflict"]
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        applied: options.apply,
        store_path: store_path.to_string_lossy().to_string(),
        edges,
        rejected_edges,
    })
}

pub fn explain_graph(vault: &Path, query: &str) -> Result<GraphExplain> {
    let graph = rebuild_graph(vault, GraphBuildOptions { apply: false })?;
    let query = query.trim().to_string();
    let matched_edge = graph.edges.iter().find(|edge| edge.id == query).cloned();
    let outgoing = graph
        .edges
        .iter()
        .filter(|edge| {
            edge.source_path == query
                || matched_edge
                    .as_ref()
                    .is_some_and(|m| m.source_path == edge.source_path)
        })
        .cloned()
        .collect::<Vec<_>>();
    let incoming = graph
        .edges
        .iter()
        .filter(|edge| {
            edge.target_path == query
                || matched_edge
                    .as_ref()
                    .is_some_and(|m| m.target_path == edge.target_path)
        })
        .cloned()
        .collect::<Vec<_>>();
    let rejected = graph
        .rejected_edges
        .into_iter()
        .filter(|edge| {
            edge.proposal_id.as_deref() == Some(query.as_str())
                || edge.source_path.as_deref() == Some(query.as_str())
                || edge.target_path.as_deref() == Some(query.as_str())
        })
        .collect::<Vec<_>>();
    Ok(GraphExplain {
        schema_version: "orderk.graph.explain.v1".to_string(),
        query,
        matched_edge,
        outgoing,
        incoming,
        rejected,
        graph_store_path: graph.store_path,
    })
}

pub fn bounded_graph_boost(base_score: f32, active_edge_count: usize) -> f32 {
    if !base_score.is_finite() || base_score >= 0.95 || active_edge_count == 0 {
        return 0.0;
    }
    let boost = (active_edge_count.min(3) as f32) * 0.01;
    boost.min(0.03)
}

fn markdown_edges(vault: &Path) -> Result<Vec<GraphEdge>> {
    let scanned = scan_vault(vault)?;
    let mut title_to_path = HashMap::new();
    let mut stem_to_path = HashMap::new();
    let mut parsed_docs = Vec::new();
    for file in scanned {
        let body = fs::read_to_string(&file.abs_path)
            .with_context(|| format!("read markdown source: {}", file.abs_path.display()))?;
        let parsed = parse_markdown(&file.path, &body)?;
        stem_to_path.insert(path_stem_key(&parsed.path), parsed.path.clone());
        if let Some(title) = parsed.title.as_deref() {
            title_to_path.insert(normalize_link_key(title), parsed.path.clone());
        }
        parsed_docs.push(parsed);
    }
    let mut edges = Vec::new();
    for doc in parsed_docs {
        for raw_link in doc.wikilinks {
            let key = normalize_link_key(&raw_link);
            let Some(target_path) = stem_to_path
                .get(&key)
                .or_else(|| title_to_path.get(&key))
                .cloned()
            else {
                continue;
            };
            if target_path == doc.path {
                continue;
            }
            edges.push(edge(EdgeDraft {
                source_path: &doc.path,
                target_path: &target_path,
                relation: GraphEdgeRelation::Supports,
                state: GraphEdgeState::Active,
                confidence: 0.78,
                source: "markdown_wikilink",
                proposal_id: None,
                evidence_paths: vec![doc.path.clone(), target_path.clone()],
                rationale: format!(
                    "Markdown wikilink {raw_link} rebuilt as an active supports edge."
                ),
            }));
        }
    }
    Ok(dedupe_edges(edges))
}

fn read_sidecar_edges(
    vault: &Path,
    audit_statuses: &HashMap<String, GraphEdgeState>,
    edges: &mut Vec<GraphEdge>,
    rejected_edges: &mut Vec<RejectedGraphEdge>,
) -> Result<()> {
    let runs_dir = vault.join(".orderk").join("sword_spirit").join("runs");
    if !runs_dir.is_dir() {
        return Ok(());
    }
    let vault_paths = scan_vault(vault)?
        .into_iter()
        .map(|file| file.path)
        .collect::<BTreeSet<_>>();
    let mut runs = fs::read_dir(&runs_dir)?.collect::<Result<Vec<_>, _>>()?;
    runs.sort_by_key(|entry| entry.path());
    for run in runs {
        if !run.file_type()?.is_dir() {
            continue;
        }
        let proposals_path = run.path().join("proposals.jsonl");
        if !proposals_path.is_file() {
            continue;
        }
        for proposal in read_proposals_jsonl(&proposals_path)? {
            let proposal_id = proposal.id.clone();
            let relation_raw = proposal.relation.clone().unwrap_or_default();
            let Some(relation) = GraphEdgeRelation::parse(&relation_raw) else {
                rejected_edges.push(rejected_edge(
                    Some(proposal_id),
                    Some(proposal.source_path),
                    proposal.target_path,
                    Some(relation_raw),
                    "relation_not_in_prd_allowlist",
                ));
                continue;
            };
            let source_path =
                match existing_vault_rel_path("source_path", &proposal.source_path, &vault_paths) {
                    Ok(path) => path,
                    Err(err) => {
                        rejected_edges.push(rejected_edge(
                            Some(proposal_id),
                            Some(proposal.source_path),
                            proposal.target_path,
                            Some(relation_raw),
                            &format!("unsafe_or_missing_source_path:{err}"),
                        ));
                        continue;
                    }
                };
            let Some(target_raw) = proposal.target_path.clone() else {
                rejected_edges.push(rejected_edge(
                    Some(proposal_id),
                    Some(source_path),
                    None,
                    Some(relation_raw),
                    "missing_target_path",
                ));
                continue;
            };
            let target_path =
                match existing_vault_rel_path("target_path", &target_raw, &vault_paths) {
                    Ok(path) => path,
                    Err(err) => {
                        rejected_edges.push(rejected_edge(
                            Some(proposal_id),
                            Some(source_path),
                            Some(target_raw),
                            Some(relation_raw),
                            &format!("unsafe_or_missing_target_path:{err}"),
                        ));
                        continue;
                    }
                };
            let mut evidence_paths = Vec::new();
            let mut evidence_error = None;
            for evidence in &proposal.evidence {
                match existing_vault_rel_path("evidence_path", &evidence.path, &vault_paths) {
                    Ok(path) => evidence_paths.push(path),
                    Err(err) => {
                        evidence_error = Some(format!("unsafe_or_missing_evidence_path:{err}"));
                        break;
                    }
                }
            }
            if let Some(reason) = evidence_error {
                rejected_edges.push(rejected_edge(
                    Some(proposal_id),
                    Some(source_path),
                    Some(target_path),
                    Some(relation_raw),
                    &reason,
                ));
                continue;
            }
            if !evidence_paths.iter().any(|path| path == &target_path) {
                rejected_edges.push(rejected_edge(
                    Some(proposal_id),
                    Some(source_path),
                    Some(target_path),
                    Some(relation_raw),
                    "target_path_not_in_evidence_set",
                ));
                continue;
            }
            let state = audit_statuses
                .get(&proposal_id)
                .copied()
                .unwrap_or(GraphEdgeState::Proposal);
            edges.push(edge(EdgeDraft {
                source_path: &source_path,
                target_path: &target_path,
                relation,
                state,
                confidence: proposal.confidence,
                source: "sword_sidecar",
                proposal_id: Some(proposal_id),
                evidence_paths,
                rationale: proposal.rationale,
            }));
        }
    }
    Ok(())
}

fn read_proposals_jsonl(path: &Path) -> Result<Vec<SwordSpiritProposal>> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut proposals = Vec::new();
    for (line_no, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        proposals.push(
            serde_json::from_str(trimmed)
                .with_context(|| format!("parse {} line {}", path.display(), line_no + 1))?,
        );
    }
    Ok(proposals)
}

fn rejected_edge(
    proposal_id: Option<String>,
    source_path: Option<String>,
    target_path: Option<String>,
    raw_relation: Option<String>,
    reason: &str,
) -> RejectedGraphEdge {
    RejectedGraphEdge {
        schema_version: "orderk.graph.rejected_edge.v1".to_string(),
        proposal_id,
        source_path,
        target_path,
        raw_relation,
        reason: reason.to_string(),
    }
}

fn normalize_vault_relative_path(label: &str, raw: &str) -> Result<String> {
    let raw = raw.trim().replace('\\', "/");
    if raw.is_empty() {
        return Err(anyhow!("unsafe graph {label}: empty path"));
    }
    let path = Path::new(&raw);
    if path.is_absolute() {
        return Err(anyhow!("unsafe graph {label}: absolute path {raw}"));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(anyhow!("unsafe graph {label}: path escapes vault {raw}"));
            }
        }
    }
    if parts.is_empty() {
        return Err(anyhow!("unsafe graph {label}: empty path"));
    }
    Ok(parts.join("/"))
}

fn existing_vault_rel_path(
    label: &str,
    raw: &str,
    vault_paths: &BTreeSet<String>,
) -> Result<String> {
    let normalized = normalize_vault_relative_path(label, raw)?;
    if vault_paths.contains(&normalized) {
        Ok(normalized)
    } else {
        Err(anyhow!("{label} missing from scanned vault: {normalized}"))
    }
}

fn ensure_plain_sidecar_dir(vault: &Path, root: &Path, label: &str) -> Result<()> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let orderk_dir = vault.join(".orderk");
    for dir in [&orderk_dir, root] {
        if let Ok(meta) = fs::symlink_metadata(dir) {
            if meta.file_type().is_symlink() {
                return Err(anyhow!(
                    "refusing to use symlinked {label} sidecar directory: {}",
                    dir.display()
                ));
            }
            if !meta.is_dir() {
                return Err(anyhow!(
                    "{label} sidecar path is not a directory: {}",
                    dir.display()
                ));
            }
        }
    }
    fs::create_dir_all(root)?;
    let canonical_root = root.canonicalize()?;
    if !canonical_root.starts_with(&vault) {
        return Err(anyhow!(
            "{label} sidecar directory escapes vault: {}",
            root.display()
        ));
    }
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
            return Err(anyhow!(
                "{label} output path is not a file: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn proposal_audit_statuses(vault: &Path) -> Result<HashMap<String, GraphEdgeState>> {
    let audit = vault.join(".orderk").join("proposals").join("audit.jsonl");
    let Ok(meta) = fs::symlink_metadata(&audit) else {
        return Ok(HashMap::new());
    };
    if meta.file_type().is_symlink() {
        return Err(anyhow!(
            "refusing to read graph proposal audit through symlink: {}",
            audit.display()
        ));
    }
    if !meta.is_file() {
        return Ok(HashMap::new());
    }
    let raw = fs::read_to_string(&audit)?;
    let mut statuses = HashMap::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let event: ProposalAuditEvent = serde_json::from_str(line)?;
        if let Some(status) = event.status.as_deref().and_then(GraphEdgeState::parse) {
            statuses.insert(event.proposal_id, status);
        }
    }
    Ok(statuses)
}

fn mark_conflicts(edges: &mut [GraphEdge]) {
    let mut active_relations: BTreeMap<(String, String), BTreeSet<GraphEdgeRelation>> =
        BTreeMap::new();
    for edge in edges
        .iter()
        .filter(|edge| edge.state == GraphEdgeState::Active)
    {
        active_relations
            .entry((edge.source_path.clone(), edge.target_path.clone()))
            .or_default()
            .insert(edge.relation);
    }
    let conflict_pairs = active_relations
        .into_iter()
        .filter_map(|(pair, relations)| (relations.len() > 1).then_some(pair))
        .collect::<BTreeSet<_>>();
    for edge in edges {
        if edge.state == GraphEdgeState::Active
            && conflict_pairs.contains(&(edge.source_path.clone(), edge.target_path.clone()))
        {
            edge.state = GraphEdgeState::Conflict;
            edge.id = edge_id(
                &edge.source_path,
                edge.relation,
                &edge.target_path,
                edge.state,
                edge.proposal_id.as_deref(),
            );
        }
    }
}

fn dedupe_edges(edges: Vec<GraphEdge>) -> Vec<GraphEdge> {
    let mut seen = BTreeSet::new();
    edges
        .into_iter()
        .filter(|edge| seen.insert(edge.id.clone()))
        .collect()
}

fn write_graph_store(
    vault: &Path,
    path: &Path,
    edges: &[GraphEdge],
    rejected: &[RejectedGraphEdge],
) -> Result<()> {
    let root = path
        .parent()
        .ok_or_else(|| anyhow!("graph store path has no parent: {}", path.display()))?;
    ensure_plain_sidecar_dir(vault, root, "graph")?;
    ensure_plain_output_file(path, "graph edges")?;
    let rejected_path = root.join("rejected_edges.jsonl");
    ensure_plain_output_file(&rejected_path, "graph rejected edges")?;
    let body = edges
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()?
        .join("\n");
    fs::write(path, format!("{body}\n"))?;
    let rejected_body = rejected
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()?
        .join("\n");
    fs::write(rejected_path, format!("{rejected_body}\n"))?;
    Ok(())
}

struct EdgeDraft<'a> {
    source_path: &'a str,
    target_path: &'a str,
    relation: GraphEdgeRelation,
    state: GraphEdgeState,
    confidence: f32,
    source: &'a str,
    proposal_id: Option<String>,
    evidence_paths: Vec<String>,
    rationale: String,
}

fn edge(draft: EdgeDraft<'_>) -> GraphEdge {
    GraphEdge {
        schema_version: "orderk.graph.edge.v1".to_string(),
        id: edge_id(
            draft.source_path,
            draft.relation,
            draft.target_path,
            draft.state,
            draft.proposal_id.as_deref(),
        ),
        source_path: draft.source_path.to_string(),
        target_path: draft.target_path.to_string(),
        relation: draft.relation,
        state: draft.state,
        confidence: if draft.confidence.is_finite() {
            draft.confidence.clamp(0.0, 1.0)
        } else {
            0.0
        },
        source: draft.source.to_string(),
        proposal_id: draft.proposal_id,
        evidence_paths: draft.evidence_paths,
        rationale: draft.rationale,
    }
}

fn edge_id(
    source_path: &str,
    relation: GraphEdgeRelation,
    target_path: &str,
    state: GraphEdgeState,
    proposal_id: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_path.as_bytes());
    hasher.update(b"\0");
    hasher.update(relation.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(target_path.as_bytes());
    hasher.update(b"\0");
    hasher.update(format!("{state:?}").as_bytes());
    hasher.update(b"\0");
    hasher.update(proposal_id.unwrap_or_default().as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("edge-{}", &digest[..16])
}

fn normalize_link_key(raw: &str) -> String {
    let before_alias = raw.split('|').next().unwrap_or(raw);
    let before_anchor = before_alias.split('#').next().unwrap_or(before_alias);
    let mut normalized = before_anchor.trim().replace('\\', "/");
    if normalized.ends_with(".md") {
        normalized.truncate(normalized.len() - 3);
    }
    normalized
        .rsplit('/')
        .next()
        .unwrap_or(normalized.as_str())
        .trim()
        .to_ascii_lowercase()
}

fn path_stem_key(path: &str) -> String {
    let path = Path::new(path);
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

#[allow(dead_code)]
fn unique_run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}", std::process::id())
}
