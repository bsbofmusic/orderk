use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

use crate::markdown::parse_markdown;
use crate::scanner::scan_vault;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdapterScanOptions {
    #[serde(default = "default_max_files")]
    pub max_files: usize,
    #[serde(default = "default_include_attachments")]
    pub include_attachments: bool,
}

impl Default for AdapterScanOptions {
    fn default() -> Self {
        Self {
            max_files: default_max_files(),
            include_attachments: default_include_attachments(),
        }
    }
}

fn default_max_files() -> usize {
    1_000
}

fn default_include_attachments() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdapterScanReport {
    pub ok: bool,
    pub schema_version: String,
    pub adapter: String,
    pub root: String,
    pub files: Vec<AdapterFile>,
    pub concepts: Vec<AdapterConcept>,
    pub tags: Vec<AdapterTag>,
    pub attachments: Vec<AdapterAttachment>,
    pub write_capability: String,
    pub raw_write_performed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdapterFile {
    pub path: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub wikilinks: Vec<String>,
    pub size_bytes: u64,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdapterConcept {
    pub path: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdapterTag {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdapterAttachment {
    pub path: String,
    pub referenced_by: Vec<String>,
    pub kind: String,
}

pub fn scan_obsidian_adapter(
    root: &Path,
    options: &AdapterScanOptions,
) -> Result<AdapterScanReport> {
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize adapter root {}", root.display()))?;
    let scanned = scan_vault(&root)?;
    let mut files = Vec::new();
    let mut concepts = Vec::new();
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();
    let mut attachments_by_path: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut warnings = Vec::new();
    record_markdown_symlink_warnings(&root, &mut warnings)?;

    for scanned_file in scanned.into_iter().take(options.max_files) {
        let rel = normalize_vault_relative(&scanned_file.path)?;
        let meta = fs::symlink_metadata(&scanned_file.abs_path)
            .with_context(|| format!("metadata for {}", rel))?;
        if meta.file_type().is_symlink() {
            warnings.push(format!("skipped symlink markdown: {rel}"));
            continue;
        }
        if !scanned_file.abs_path.starts_with(&root) {
            warnings.push(format!("skipped path outside adapter root: {rel}"));
            continue;
        }
        let raw = fs::read_to_string(&scanned_file.abs_path)
            .with_context(|| format!("read markdown metadata for {}", rel))?;
        let parsed = parse_markdown(&rel, &raw)?;
        for tag in &parsed.tags {
            *tags.entry(tag.clone()).or_insert(0) += 1;
        }
        if options.include_attachments {
            for attachment in extract_obsidian_attachments(&raw) {
                let Ok(attachment_path) = normalize_vault_relative(&attachment) else {
                    warnings.push(format!(
                        "skipped unsafe attachment reference in {rel}: {attachment}"
                    ));
                    continue;
                };
                let abs = root.join(&attachment_path);
                if !abs.exists() {
                    warnings.push(format!(
                        "missing attachment referenced by {rel}: {attachment_path}"
                    ));
                    continue;
                }
                if fs::symlink_metadata(&abs)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(true)
                {
                    warnings.push(format!(
                        "skipped symlink attachment referenced by {rel}: {attachment_path}"
                    ));
                    continue;
                }
                attachments_by_path
                    .entry(attachment_path)
                    .or_default()
                    .insert(rel.clone());
            }
        }
        let adapter_file = AdapterFile {
            path: rel.clone(),
            title: parsed.title.clone(),
            tags: parsed.tags.clone(),
            wikilinks: parsed.wikilinks.clone(),
            size_bytes: scanned_file.size,
            hash: scanned_file.hash,
        };
        if is_concept_path(&rel) {
            concepts.push(AdapterConcept {
                path: rel.clone(),
                title: parsed.title,
                tags: parsed.tags,
            });
        }
        files.push(adapter_file);
    }

    concepts.sort_by(|a, b| a.path.cmp(&b.path));
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let tags = tags
        .into_iter()
        .map(|(name, count)| AdapterTag { name, count })
        .collect::<Vec<_>>();
    let attachments = attachments_by_path
        .into_iter()
        .map(|(path, refs)| AdapterAttachment {
            kind: attachment_kind(&path).to_string(),
            path,
            referenced_by: refs.into_iter().collect(),
        })
        .collect::<Vec<_>>();

    Ok(AdapterScanReport {
        ok: true,
        schema_version: "orderk.adapter.obsidian.v1".to_string(),
        adapter: "obsidian".to_string(),
        root: root.to_string_lossy().to_string(),
        files,
        concepts,
        tags,
        attachments,
        write_capability: "disabled".to_string(),
        raw_write_performed: false,
        warnings,
    })
}

fn record_markdown_symlink_warnings(root: &Path, warnings: &mut Vec<String>) -> Result<()> {
    for entry in WalkDir::new(root).follow_links(false).into_iter() {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        if !entry.file_type().is_symlink() {
            continue;
        }
        let rel = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        warnings.push(format!("skipped symlink markdown: {rel}"));
    }
    Ok(())
}

fn normalize_vault_relative(raw: &str) -> Result<String> {
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(anyhow!("adapter path must be vault-relative"));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(anyhow!("adapter path escapes vault root"));
            }
        }
    }
    if parts.is_empty() || parts.iter().any(|part| part.is_empty()) {
        return Err(anyhow!("adapter path must not be empty"));
    }
    Ok(parts.join("/"))
}

fn extract_obsidian_attachments(source: &str) -> Vec<String> {
    let re = Regex::new(r"!\[\[([^\]]+)\]\]").unwrap();
    re.captures_iter(source)
        .filter_map(|capture| capture.get(1))
        .map(|m| {
            m.as_str()
                .split('|')
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn is_concept_path(rel: &str) -> bool {
    rel.contains("/concepts/") || rel.starts_with("concepts/") || rel.starts_with("wiki/concepts/")
}

fn attachment_kind(path: &str) -> &'static str {
    match PathBuf::from(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => "image",
        "pdf" => "pdf",
        "mp3" | "wav" | "m4a" | "flac" => "audio",
        _ => "file",
    }
}
