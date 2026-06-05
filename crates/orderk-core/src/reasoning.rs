use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path};

use crate::markdown::parse_markdown;
use crate::scanner::scan_vault;

const REASONING_TRIGGER_THRESHOLD: f32 = 0.60;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReasoningOptions {
    pub query: String,
    pub context_paths: Vec<String>,
    pub allow_llm: bool,
    pub confidence_hint: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReasoningReport {
    pub ok: bool,
    pub schema_version: String,
    pub mode: String,
    pub query: String,
    pub reasoning_triggered: bool,
    pub trigger_reasons: Vec<String>,
    pub llm_allowed: bool,
    pub llm_calls: usize,
    pub llm_invocation: String,
    pub evidence_used: Vec<ReasoningEvidence>,
    pub relations_activated: Vec<ReasoningRelation>,
    pub conclusion: String,
    pub confidence: f32,
    pub boundary: ReasoningBoundary,
    pub suggested_patch: SuggestedPatch,
    pub mutation_policy: String,
    pub raw_unchanged: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReasoningEvidence {
    pub path: String,
    pub title: Option<String>,
    pub evidence_kind: String,
    pub excerpt: String,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReasoningRelation {
    pub relation: String,
    pub source_path: String,
    pub target_path: String,
    pub state: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReasoningBoundary {
    pub evidence_only: bool,
    pub direct_write_allowed: bool,
    pub raw_write_allowed: bool,
    pub wiki_write_allowed: bool,
    pub graph_write_allowed: bool,
    pub suggested_patch_route: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SuggestedPatch {
    pub status: String,
    pub route: String,
    pub apply_allowed: bool,
    pub target_path: Option<String>,
    pub relation: Option<String>,
    pub summary: String,
    pub patch_text: Option<String>,
}

pub fn reason_about_vault(vault: &Path, options: ReasoningOptions) -> Result<ReasoningReport> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let scanned = scan_vault(&vault)?;
    let by_path = scanned
        .iter()
        .map(|file| (file.path.clone(), file.clone()))
        .collect::<BTreeMap<_, _>>();
    let evidence_paths = select_evidence_paths(&vault, &options, &by_path)?;
    let trigger_reasons = reasoning_trigger_reasons(&options, &evidence_paths);
    let reasoning_triggered = !trigger_reasons.is_empty();
    let evidence_used = if reasoning_triggered {
        evidence_paths
            .iter()
            .filter_map(|path| evidence_for_path(path, &by_path).transpose())
            .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let relations_activated = if reasoning_triggered {
        activated_relations(&vault, &evidence_used)?
    } else {
        Vec::new()
    };
    let llm_invocation = if !reasoning_triggered {
        "not_called_no_trigger"
    } else if options.allow_llm {
        "not_called_evidence_only"
    } else {
        "not_allowed_evidence_only"
    }
    .to_string();
    let confidence = if reasoning_triggered {
        (0.45 + evidence_used.len() as f32 * 0.08 + relations_activated.len() as f32 * 0.05)
            .clamp(0.0, 0.86)
    } else {
        0.0
    };
    let conclusion = if reasoning_triggered {
        let subjects = evidence_used
            .iter()
            .map(|evidence| evidence.path.as_str())
            .take(3)
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "Evidence-only reasoning activated for {}. The result is a bounded conclusion from {} and must be routed as a proposal before any write.",
            trigger_reasons.join("+"),
            if subjects.is_empty() { "scanned vault evidence" } else { &subjects }
        )
    } else {
        "No active reasoning trigger matched; default retrieval/search path stays no-LLM and no-write."
            .to_string()
    };
    let suggested_patch = if reasoning_triggered {
        SuggestedPatch {
            status: "proposal_required".to_string(),
            route: "proposal_flow_only".to_string(),
            apply_allowed: false,
            target_path: evidence_used.first().map(|evidence| evidence.path.clone()),
            relation: relations_activated.first().map(|relation| relation.relation.clone()),
            summary: "Create a proposal with the evidence_used and relations_activated fields; do not apply directly.".to_string(),
            patch_text: None,
        }
    } else {
        SuggestedPatch {
            status: "not_suggested_no_trigger".to_string(),
            route: "proposal_flow_only".to_string(),
            apply_allowed: false,
            target_path: None,
            relation: None,
            summary: "No patch suggested because reasoning did not trigger.".to_string(),
            patch_text: None,
        }
    };
    Ok(ReasoningReport {
        ok: true,
        schema_version: "orderk.reasoning.result.v1".to_string(),
        mode: "evidence_only".to_string(),
        query: options.query,
        reasoning_triggered,
        trigger_reasons,
        llm_allowed: options.allow_llm,
        llm_calls: 0,
        llm_invocation,
        evidence_used,
        relations_activated,
        conclusion,
        confidence,
        boundary: ReasoningBoundary {
            evidence_only: true,
            direct_write_allowed: false,
            raw_write_allowed: false,
            wiki_write_allowed: false,
            graph_write_allowed: false,
            suggested_patch_route: "proposals".to_string(),
        },
        suggested_patch,
        mutation_policy: "no_direct_writes".to_string(),
        raw_unchanged: true,
        warnings: Vec::new(),
    })
}

fn reasoning_trigger_reasons(options: &ReasoningOptions, evidence_paths: &[String]) -> Vec<String> {
    let query = options.query.to_lowercase();
    let mut reasons = Vec::new();
    if contains_any(
        &query,
        &[
            "判断",
            "取舍",
            "复盘",
            "架构",
            "综合",
            "推理",
            "结论",
            "为什么",
            "tradeoff",
            "trade-off",
            "architecture",
            "reason",
            "synthesis",
            "decision",
            "review",
        ],
    ) {
        reasons.push("explicit_high_level_intent".to_string());
    }
    if options
        .confidence_hint
        .is_some_and(|confidence| confidence < REASONING_TRIGGER_THRESHOLD)
    {
        reasons.push("low_confidence".to_string());
    }
    if contains_any(
        &query,
        &["冲突", "矛盾", "conflict", "contradict", "disagree"],
    ) {
        reasons.push("conflict".to_string());
    }
    if evidence_paths.len() >= 2
        && contains_any(
            &query,
            &[
                "和", "与", "关系", "关联", "cross", "between", "compare", "versus",
            ],
        )
    {
        reasons.push("cross_concept_synthesis".to_string());
    }
    if evidence_paths
        .iter()
        .any(|path| path.starts_with("raw/") || path.starts_with("source/"))
        && contains_any(&query, &["raw", "未整理", "未消化", "undigested", "source"])
    {
        reasons.push("undigested_raw".to_string());
    }
    reasons.sort();
    reasons.dedup();
    reasons
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn select_evidence_paths(
    vault: &Path,
    options: &ReasoningOptions,
    by_path: &BTreeMap<String, crate::models::ScannedFile>,
) -> Result<Vec<String>> {
    if !options.context_paths.is_empty() {
        let mut out = Vec::new();
        for raw in &options.context_paths {
            let normalized = normalize_reasoning_context_path(raw)?;
            if !by_path.contains_key(&normalized) {
                return Err(anyhow!("missing reasoning context path: {normalized}"));
            }
            out.push(normalized);
        }
        out.sort();
        out.dedup();
        return Ok(out);
    }

    let query = options.query.to_lowercase();
    let mut scored = Vec::new();
    for (rel, file) in by_path {
        let raw = fs::read_to_string(&file.abs_path)
            .unwrap_or_default()
            .to_lowercase();
        let mut score = 0usize;
        if !query.is_empty() && raw.contains(&query) {
            score += 3;
        }
        for token in query.split_whitespace().filter(|token| token.len() >= 2) {
            if rel.to_lowercase().contains(token) || raw.contains(token) {
                score += 1;
            }
        }
        if score > 0 {
            scored.push((score, rel.clone()));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let selected = scored
        .into_iter()
        .map(|(_, rel)| rel)
        .take(5)
        .collect::<Vec<_>>();
    if selected.is_empty() && vault.is_dir() {
        Ok(by_path.keys().take(3).cloned().collect())
    } else {
        Ok(selected)
    }
}

fn normalize_reasoning_context_path(raw: &str) -> Result<String> {
    let trimmed = raw.trim().replace('\\', "/");
    if trimmed.is_empty() {
        return Err(anyhow!("unsafe reasoning context path: empty"));
    }
    let path = Path::new(&trimmed);
    if path.is_absolute() {
        return Err(anyhow!(
            "unsafe reasoning context path: absolute path {trimmed}"
        ));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(anyhow!("unsafe reasoning context path: {trimmed}"));
            }
        }
    }
    if parts.is_empty() {
        return Err(anyhow!("unsafe reasoning context path: empty"));
    }
    Ok(parts.join("/"))
}

fn evidence_for_path(
    rel: &str,
    by_path: &BTreeMap<String, crate::models::ScannedFile>,
) -> Result<Option<ReasoningEvidence>> {
    let Some(file) = by_path.get(rel) else {
        return Ok(None);
    };
    let raw = fs::read_to_string(&file.abs_path).with_context(|| format!("read evidence {rel}"))?;
    let parsed = parse_markdown(rel, &raw)?;
    Ok(Some(ReasoningEvidence {
        path: rel.to_string(),
        title: parsed.title.as_deref().map(sanitize_excerpt),
        evidence_kind: if rel.starts_with("raw/") || rel.starts_with("source/") {
            "undigested_raw"
        } else {
            "markdown_note"
        }
        .to_string(),
        excerpt: sanitize_excerpt(&parsed.body),
        hash: file.hash.clone(),
    }))
}

fn sanitize_excerpt(raw: &str) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut sanitized = collapsed;
    for (pattern, replacement) in [
        (
            r"(?i)\b(authorization\s*:\s*bearer)\s+[^\s,;]+",
            "$1 [REDACTED]",
        ),
        (
            r"(?i)\b(password|passwd|secret|api[_-]?key|token|authorization|bearer)\s*[:=]\s*[^\s,;]+",
            "$1=[REDACTED]",
        ),
        (
            r"(?i)\b([A-Z0-9_]*(?:API[_-]?KEY|TOKEN|SECRET|PASSWORD)[A-Z0-9_]*)\s*=\s*[^\s,;]+",
            "$1=[REDACTED]",
        ),
        (r"\bsk-[A-Za-z0-9][A-Za-z0-9._-]{8,}\b", "[REDACTED]"),
    ] {
        sanitized = Regex::new(pattern)
            .expect("reasoning redaction regex compiles")
            .replace_all(&sanitized, replacement)
            .into_owned();
    }
    let mut excerpt = String::new();
    for token in sanitized.split_whitespace() {
        let token = if looks_like_secret(token) {
            "[REDACTED]"
        } else {
            token
        };
        if !excerpt.is_empty() {
            excerpt.push(' ');
        }
        excerpt.push_str(token);
        if excerpt.chars().count() >= 180 {
            break;
        }
    }
    excerpt.chars().take(180).collect()
}

fn looks_like_secret(token: &str) -> bool {
    let lower = token.to_lowercase();
    lower.starts_with("sk-")
        || lower.starts_with("ghp_")
        || lower.starts_with("github_pat_")
        || (lower.contains("token") && token.len() > 16)
}

fn activated_relations(
    vault: &Path,
    evidence_used: &[ReasoningEvidence],
) -> Result<Vec<ReasoningRelation>> {
    let evidence_paths = evidence_used
        .iter()
        .map(|evidence| evidence.path.as_str())
        .collect::<HashSet<_>>();
    let mut out = Vec::new();
    let graph_path = vault.join(".orderk").join("graph").join("edges.jsonl");
    if graph_path.is_file() {
        let raw = fs::read_to_string(&graph_path)?;
        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let value: Value = serde_json::from_str(line)?;
            let source = value
                .get("source_path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let target = value
                .get("target_path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if evidence_paths.contains(source) || evidence_paths.contains(target) {
                out.push(ReasoningRelation {
                    relation: value
                        .get("relation")
                        .and_then(Value::as_str)
                        .unwrap_or("supports")
                        .to_string(),
                    source_path: source.to_string(),
                    target_path: target.to_string(),
                    state: value
                        .get("state")
                        .and_then(Value::as_str)
                        .unwrap_or("active")
                        .to_string(),
                    confidence: value
                        .get("confidence")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.5) as f32,
                });
            }
        }
    }
    if out.is_empty() && evidence_used.len() >= 2 {
        out.push(ReasoningRelation {
            relation: "supports".to_string(),
            source_path: evidence_used[0].path.clone(),
            target_path: evidence_used[1].path.clone(),
            state: "proposal".to_string(),
            confidence: 0.42,
        });
    }
    Ok(out)
}
