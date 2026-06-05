use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::scanner::scan_vault;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DigestOptions {
    pub profile: String,
    pub apply: bool,
    pub resume: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DigestFileState {
    pub hash: String,
    pub mtime: i64,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DigestState {
    pub schema_version: String,
    pub profile: String,
    pub files: BTreeMap<String, DigestFileState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DigestReport {
    pub schema_version: String,
    pub vault: String,
    pub profile: String,
    pub dry_run: bool,
    pub apply: bool,
    pub resume: bool,
    pub lock_path: String,
    pub state_path: String,
    pub run_id: Option<String>,
    pub changed_paths: Vec<String>,
    pub removed_paths: Vec<String>,
    pub unchanged_paths: Vec<String>,
    pub scanned_files: usize,
    pub state_written: bool,
}

pub fn digest_vault(vault: &Path, options: DigestOptions) -> Result<DigestReport> {
    let vault = vault
        .canonicalize()
        .with_context(|| format!("vault path not found: {}", vault.display()))?;
    let root = vault.join(".orderk").join("digest");
    let lock_path = root.join("digest.lock");
    let state_path = root.join("state.json");
    if options.apply {
        prepare_digest_root(&vault, &root)?;
        let _lock = create_digest_lock(&lock_path, options.resume)?;
        let previous = read_state(&state_path)?;
        let current = current_state(&vault, &options.profile)?;
        let (changed_paths, removed_paths, unchanged_paths) =
            diff_states(previous.as_ref(), &current);
        let id = format!(
            "{}-{}",
            Utc::now().format("%Y%m%dT%H%M%SZ"),
            std::process::id()
        );
        write_state(&state_path, &current)?;
        let runs_root = root.join("runs");
        prepare_child_dir(&runs_root, "digest runs")?;
        let runs_dir = runs_root.join(&id);
        prepare_child_dir(&runs_dir, "digest run")?;
        let preliminary = DigestReport {
            schema_version: "orderk.digest.report.v1".to_string(),
            vault: vault.to_string_lossy().to_string(),
            profile: options.profile.clone(),
            dry_run: false,
            apply: true,
            resume: options.resume,
            lock_path: lock_path.to_string_lossy().to_string(),
            state_path: state_path.to_string_lossy().to_string(),
            run_id: Some(id.clone()),
            changed_paths: changed_paths.clone(),
            removed_paths: removed_paths.clone(),
            unchanged_paths: unchanged_paths.clone(),
            scanned_files: current.files.len(),
            state_written: true,
        };
        let summary_path = runs_dir.join("summary.json");
        ensure_plain_output_file(&summary_path, "digest run summary")?;
        fs::write(
            summary_path,
            serde_json::to_string_pretty(&preliminary)? + "\n",
        )?;
        fs::remove_file(&lock_path)?;
        return Ok(preliminary);
    }

    let previous = read_state(&state_path)?;
    let current = current_state(&vault, &options.profile)?;
    let (changed_paths, removed_paths, unchanged_paths) = diff_states(previous.as_ref(), &current);
    Ok(DigestReport {
        schema_version: "orderk.digest.report.v1".to_string(),
        vault: vault.to_string_lossy().to_string(),
        profile: options.profile,
        dry_run: true,
        apply: false,
        resume: options.resume,
        lock_path: lock_path.to_string_lossy().to_string(),
        state_path: state_path.to_string_lossy().to_string(),
        run_id: None,
        changed_paths,
        removed_paths,
        unchanged_paths,
        scanned_files: current.files.len(),
        state_written: false,
    })
}

fn current_state(vault: &Path, profile: &str) -> Result<DigestState> {
    let mut files = BTreeMap::new();
    for file in scan_vault(vault)? {
        files.insert(
            file.path,
            DigestFileState {
                hash: file.hash,
                mtime: file.mtime,
                size: file.size,
            },
        );
    }
    Ok(DigestState {
        schema_version: "orderk.digest.state.v1".to_string(),
        profile: profile.to_string(),
        files,
    })
}

fn diff_states(
    previous: Option<&DigestState>,
    current: &DigestState,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut changed = Vec::new();
    let mut removed = Vec::new();
    let mut unchanged = Vec::new();
    let Some(previous) = previous else {
        return (current.files.keys().cloned().collect(), removed, unchanged);
    };
    let profile_changed = previous.profile != current.profile;
    for (path, state) in &current.files {
        if profile_changed || previous.files.get(path) != Some(state) {
            changed.push(path.clone());
        } else {
            unchanged.push(path.clone());
        }
    }
    for path in previous.files.keys() {
        if !current.files.contains_key(path) {
            removed.push(path.clone());
        }
    }
    (changed, removed, unchanged)
}

fn read_state(path: &Path) -> Result<Option<DigestState>> {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Ok(None);
    };
    if meta.file_type().is_symlink() {
        return Err(anyhow!(
            "refusing to read digest state through symlink: {}",
            path.display()
        ));
    }
    if !meta.is_file() {
        return Err(anyhow!(
            "digest state path is not a file: {}",
            path.display()
        ));
    }
    let raw = fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str(&raw)?))
}

fn write_state(path: &Path, state: &DigestState) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("digest state path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)?;
    ensure_plain_output_file(path, "digest state")?;
    let tmp: PathBuf = path.with_extension("json.tmp");
    ensure_plain_output_file(&tmp, "digest temporary state")?;
    fs::write(&tmp, serde_json::to_string_pretty(state)? + "\n")?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn prepare_digest_root(vault: &Path, root: &Path) -> Result<()> {
    let orderk_dir = vault.join(".orderk");
    for dir in [&orderk_dir, root] {
        if let Ok(meta) = fs::symlink_metadata(dir) {
            if meta.file_type().is_symlink() {
                return Err(anyhow!(
                    "refusing to use symlinked digest sidecar directory: {}",
                    dir.display()
                ));
            }
            if !meta.is_dir() {
                return Err(anyhow!(
                    "digest sidecar path is not a directory: {}",
                    dir.display()
                ));
            }
        }
    }
    fs::create_dir_all(root)?;
    let canonical_root = root.canonicalize()?;
    if !canonical_root.starts_with(vault) {
        return Err(anyhow!(
            "digest sidecar directory escapes vault: {}",
            root.display()
        ));
    }
    Ok(())
}

fn prepare_child_dir(path: &Path, label: &str) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(anyhow!(
                "refusing to use symlinked {label} directory: {}",
                path.display()
            ));
        }
        if !meta.is_dir() {
            return Err(anyhow!(
                "{label} path is not a directory: {}",
                path.display()
            ));
        }
    }
    fs::create_dir_all(path)?;
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
            return Err(anyhow!("{label} path is not a file: {}", path.display()));
        }
    }
    Ok(())
}

fn create_digest_lock(path: &Path, resume: bool) -> Result<File> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(anyhow!(
                "refusing to use digest lock through symlink: {}",
                path.display()
            ));
        }
        if resume {
            fs::remove_file(path)?;
        } else {
            return Err(anyhow!(
                "digest lock exists at {}; pass --resume to continue a previous run",
                path.display()
            ));
        }
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create digest lock atomically: {}", path.display()))?;
    writeln!(file, "pid={}", std::process::id())?;
    Ok(file)
}
