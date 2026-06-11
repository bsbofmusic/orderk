use crate::models::ScannedFile;
use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Component, Path};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub fn scan_vault(vault: &Path) -> Result<Vec<ScannedFile>> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let mut files = Vec::new();
    for entry in WalkDir::new(&vault).follow_links(false).into_iter() {
        let entry = entry?;
        let path = entry.path();
        if should_skip(path) {
            continue;
        }
        if !entry.file_type().is_file() || path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let rel = path
            .strip_prefix(&vault)?
            .to_string_lossy()
            .replace('\\', "/");
        let metadata = entry.metadata()?;
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let hash = hex::encode(Sha256::digest(&bytes));
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default();
        files.push(ScannedFile {
            path: rel,
            abs_path: path.to_path_buf(),
            mtime,
            size: metadata.len(),
            hash,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

pub fn scan_vault_paths(vault: &Path, rel_paths: &[String]) -> Result<Vec<ScannedFile>> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let mut files = Vec::new();
    for raw in rel_paths {
        let rel = normalize_vault_relative_path(raw)?;
        let abs = vault.join(&rel);
        if should_skip(&abs) {
            return Err(anyhow!("refusing to index skipped vault path: {rel}"));
        }
        let metadata = fs::symlink_metadata(&abs)
            .with_context(|| format!("indexed path not found: {}", abs.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(anyhow!("refusing to index symlinked vault path: {rel}"));
        }
        if !metadata.is_file() || abs.extension().and_then(|s| s.to_str()) != Some("md") {
            return Err(anyhow!("indexed path is not a Markdown file: {rel}"));
        }
        let canonical = abs
            .canonicalize()
            .with_context(|| format!("canonicalize indexed path: {}", abs.display()))?;
        if !canonical.starts_with(&vault) {
            return Err(anyhow!("indexed path escapes vault: {rel}"));
        }
        files.push(scanned_file_for_path(&vault, &canonical, &rel)?);
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    Ok(files)
}

fn scanned_file_for_path(vault: &Path, path: &Path, rel: &str) -> Result<ScannedFile> {
    let metadata = fs::metadata(path)?;
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let hash = hex::encode(Sha256::digest(&bytes));
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    let rel = if rel.is_empty() {
        path.strip_prefix(vault)?
            .to_string_lossy()
            .replace('\\', "/")
    } else {
        rel.to_string()
    };
    Ok(ScannedFile {
        path: rel,
        abs_path: path.to_path_buf(),
        mtime,
        size: metadata.len(),
        hash,
    })
}

fn normalize_vault_relative_path(raw: &str) -> Result<String> {
    let path = Path::new(raw);
    if raw.trim().is_empty() || path.is_absolute() {
        return Err(anyhow!(
            "indexed path must be a non-empty vault-relative path"
        ));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(anyhow!("indexed path must not escape vault: {raw}"));
            }
        }
    }
    if parts.is_empty() {
        return Err(anyhow!("indexed path must name a Markdown file"));
    }
    Ok(parts.join("/"))
}

fn should_skip(path: &Path) -> bool {
    path.components().any(|component| {
        if let Component::Normal(name) = component {
            let s = name.to_string_lossy();
            matches!(
                s.as_ref(),
                ".obsidian" | ".orderk" | ".trash" | ".git" | "node_modules" | "target"
            )
        } else {
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dir.push(format!("orderk-scanner-{}-{}", std::process::id(), unique));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scanner_ignores_obsidian_and_non_markdown() {
        let dir = temp_dir();
        fs::create_dir_all(dir.join(".obsidian")).unwrap();
        fs::write(dir.join("a.md"), "# A").unwrap();
        fs::write(dir.join("b.txt"), "B").unwrap();
        fs::write(dir.join(".obsidian/ignored.md"), "# I").unwrap();
        let got = scan_vault(&dir).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "a.md");
        let _ = fs::remove_dir_all(&dir);
    }
}
