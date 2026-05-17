use crate::api::status;
use crate::index::IndexStore;
use anyhow::{anyhow, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

pub const CAPSULE_SCHEMA_VERSION: &str = "orderk.capsule.v1";
pub const CAPSULE_INSPECTION_SCHEMA_VERSION: &str = "orderk.capsule_inspection.v1";
pub const CAPSULE_ARTIFACT_KIND: &str = "orderk.sqlite_index";
pub const MAX_CAPSULE_MANIFEST_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapsuleManifest {
    pub schema_version: String,
    pub artifact: CapsuleArtifact,
    pub profile: CapsuleProfile,
    pub stats: CapsuleStats,
    pub source: CapsuleSource,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapsuleArtifact {
    pub kind: String,
    pub db: String,
    pub size_bytes: u64,
    pub sha256: String,
    #[serde(default)]
    pub files: Vec<CapsuleArtifactFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapsuleArtifactFile {
    pub role: String,
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapsuleProfile {
    pub schema_version: String,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub embedding_dim: usize,
    pub vector_backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapsuleStats {
    pub notes: usize,
    pub chunks: usize,
    pub embeddings: usize,
    pub fts_enabled: bool,
    pub vector_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CapsuleSource {
    pub vault: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapsuleInspection {
    pub schema_version: String,
    pub ok: bool,
    pub manifest: CapsuleManifest,
    pub checks: Vec<CapsuleCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapsuleCheck {
    pub component: String,
    pub ok: bool,
    pub message: String,
    pub details: serde_json::Value,
}

impl CapsuleCheck {
    fn ok(
        component: impl Into<String>,
        message: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        Self {
            component: component.into(),
            ok: true,
            message: message.into(),
            details,
        }
    }

    fn fail(
        component: impl Into<String>,
        message: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        Self {
            component: component.into(),
            ok: false,
            message: message.into(),
            details,
        }
    }
}

pub fn export_capsule_manifest(
    db_path: &Path,
    vault_path: Option<&Path>,
) -> Result<CapsuleManifest> {
    let (profile, stats) = current_profile_and_stats(db_path)?;
    let artifact = collect_artifact(db_path)?;
    Ok(CapsuleManifest {
        schema_version: CAPSULE_SCHEMA_VERSION.to_string(),
        artifact,
        profile,
        stats,
        source: CapsuleSource {
            vault: vault_path.map(|path| path.to_string_lossy().to_string()),
        },
        created_at: chrono::Utc::now().to_rfc3339(),
    })
}

pub fn write_capsule_manifest(
    db_path: &Path,
    vault_path: Option<&Path>,
    out_path: &Path,
) -> Result<CapsuleManifest> {
    validate_capsule_output_path(db_path, vault_path, out_path)?;
    let manifest = export_capsule_manifest(db_path, vault_path)?;
    validate_capsule_output_path(db_path, vault_path, out_path)?;
    let payload = serde_json::to_vec_pretty(&manifest)?;
    write_file_without_following_symlink(out_path, &payload)?;
    Ok(manifest)
}

pub fn inspect_capsule_manifest(
    manifest_path: &Path,
    db_path: Option<&Path>,
) -> Result<CapsuleInspection> {
    let raw = read_capsule_manifest_bounded(manifest_path)?;
    let manifest: CapsuleManifest = serde_json::from_slice(&raw).with_context(|| {
        format!(
            "failed to parse capsule manifest JSON: {}",
            manifest_path.display()
        )
    })?;
    let mut checks = Vec::new();
    inspect_manifest_artifact_consistency(&manifest, &mut checks);
    if manifest.schema_version == CAPSULE_SCHEMA_VERSION {
        checks.push(CapsuleCheck::ok(
            "manifest_schema",
            "capsule manifest schema is supported",
            json!({"schema_version": manifest.schema_version}),
        ));
    } else {
        checks.push(CapsuleCheck::fail(
            "manifest_schema",
            "unsupported capsule manifest schema",
            json!({"schema_version": manifest.schema_version, "expected": CAPSULE_SCHEMA_VERSION}),
        ));
    }
    if manifest.artifact.kind == CAPSULE_ARTIFACT_KIND {
        checks.push(CapsuleCheck::ok(
            "artifact_kind",
            "capsule artifact kind is supported",
            json!({"kind": manifest.artifact.kind}),
        ));
    } else {
        checks.push(CapsuleCheck::fail(
            "artifact_kind",
            "unsupported capsule artifact kind",
            json!({"kind": manifest.artifact.kind, "expected": CAPSULE_ARTIFACT_KIND}),
        ));
    }

    if let Some(db) = db_path {
        inspect_db_profile_and_stats(db, &manifest, &mut checks);
        inspect_db_payload(db, &manifest, &mut checks);
    } else {
        checks.push(CapsuleCheck::fail(
            "db_payload",
            "no DB path supplied for payload verification",
            json!({"expected_db": manifest.artifact.db}),
        ));
    }

    let ok = checks.iter().all(|check| check.ok);
    Ok(CapsuleInspection {
        schema_version: CAPSULE_INSPECTION_SCHEMA_VERSION.to_string(),
        ok,
        manifest,
        checks,
    })
}

fn inspect_manifest_artifact_consistency(
    manifest: &CapsuleManifest,
    checks: &mut Vec<CapsuleCheck>,
) {
    let expected_size: u64 = manifest
        .artifact
        .files
        .iter()
        .map(|file| file.size_bytes)
        .sum();
    let expected_sha256 = aggregate_artifact_hash(&manifest.artifact.files);
    let main_count = manifest
        .artifact
        .files
        .iter()
        .filter(|file| file.role == "main")
        .count();
    let roles_known = manifest
        .artifact
        .files
        .iter()
        .all(|file| matches!(file.role.as_str(), "main" | "wal" | "shm"));
    let hashes_well_formed = manifest.artifact.files.iter().all(|file| {
        file.sha256.len() == 64 && file.sha256.chars().all(|ch| ch.is_ascii_hexdigit())
    });
    let ok = !manifest.artifact.files.is_empty()
        && main_count == 1
        && roles_known
        && hashes_well_formed
        && expected_size == manifest.artifact.size_bytes
        && expected_sha256 == manifest.artifact.sha256;
    if ok {
        checks.push(CapsuleCheck::ok(
            "artifact_manifest",
            "artifact file manifest is internally consistent",
            json!({"files": manifest.artifact.files.len(), "size_bytes": manifest.artifact.size_bytes, "sha256": manifest.artifact.sha256}),
        ));
    } else {
        checks.push(CapsuleCheck::fail(
            "artifact_manifest",
            "artifact file manifest is not internally consistent",
            json!({
                "actual": {"size_bytes": manifest.artifact.size_bytes, "sha256": manifest.artifact.sha256, "main_count": main_count, "roles_known": roles_known, "hashes_well_formed": hashes_well_formed},
                "expected": {"size_bytes": expected_size, "sha256": expected_sha256, "main_count": 1},
            }),
        ));
    }
}

fn inspect_db_payload(db_path: &Path, manifest: &CapsuleManifest, checks: &mut Vec<CapsuleCheck>) {
    match collect_artifact(db_path) {
        Ok(actual) => {
            if actual.size_bytes == manifest.artifact.size_bytes {
                checks.push(CapsuleCheck::ok(
                    "db_size",
                    "DB artifact size matches manifest",
                    json!({"actual": actual.size_bytes, "expected": manifest.artifact.size_bytes}),
                ));
            } else {
                checks.push(CapsuleCheck::fail(
                    "db_size",
                    "DB artifact size does not match manifest",
                    json!({"actual": actual.size_bytes, "expected": manifest.artifact.size_bytes}),
                ));
            }
            if actual.sha256 == manifest.artifact.sha256 {
                checks.push(CapsuleCheck::ok(
                    "db_checksum",
                    "DB artifact checksum matches manifest",
                    json!({"sha256": actual.sha256, "files": actual.files}),
                ));
            } else {
                checks.push(CapsuleCheck::fail(
                    "db_checksum",
                    "DB artifact checksum does not match manifest",
                    json!({
                        "actual": {"sha256": actual.sha256, "files": actual.files},
                        "expected": {"sha256": manifest.artifact.sha256, "files": manifest.artifact.files},
                    }),
                ));
            }
        }
        Err(err) => checks.push(CapsuleCheck::fail(
            "db_payload",
            format!("failed to inspect DB payload: {err}"),
            json!({"db": db_path.to_string_lossy()}),
        )),
    }
}

fn inspect_db_profile_and_stats(
    db_path: &Path,
    manifest: &CapsuleManifest,
    checks: &mut Vec<CapsuleCheck>,
) {
    match current_profile_and_stats(db_path) {
        Ok((profile, stats)) => {
            if profile == manifest.profile {
                checks.push(CapsuleCheck::ok(
                    "profile",
                    "DB embedding/schema profile matches manifest",
                    json!({"profile": manifest.profile}),
                ));
            } else {
                checks.push(CapsuleCheck::fail(
                    "profile",
                    "DB embedding/schema profile differs from manifest",
                    json!({"actual": profile, "expected": manifest.profile}),
                ));
            }

            if stats == manifest.stats {
                checks.push(CapsuleCheck::ok(
                    "stats",
                    "DB stats match manifest",
                    json!({"stats": manifest.stats}),
                ));
            } else {
                checks.push(CapsuleCheck::fail(
                    "stats",
                    "DB stats differ from manifest",
                    json!({"actual": stats, "expected": manifest.stats}),
                ));
            }
        }
        Err(err) => checks.push(CapsuleCheck::fail(
            "db_status",
            format!("failed to inspect DB status: {err}"),
            json!({"db": db_path.to_string_lossy()}),
        )),
    }
}

fn current_profile_and_stats(db_path: &Path) -> Result<(CapsuleProfile, CapsuleStats)> {
    let current = status(db_path).with_context(|| {
        format!(
            "failed to read orderk status for capsule export: {}",
            db_path.display()
        )
    })?;
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open orderk DB settings: {}", db_path.display()))?;
    let settings = IndexStore::load_settings(&conn)?;
    let schema_version = settings
        .get("schema_version")
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    Ok((
        CapsuleProfile {
            schema_version,
            embedding_provider: current.embedding_provider,
            embedding_model: current.embedding_model,
            embedding_dim: current.embedding_dim,
            vector_backend: current.vector_backend,
        },
        CapsuleStats {
            notes: current.notes,
            chunks: current.chunks,
            embeddings: current.embeddings,
            fts_enabled: current.fts_enabled,
            vector_enabled: current.vector_enabled,
        },
    ))
}

fn collect_artifact(db_path: &Path) -> Result<CapsuleArtifact> {
    let metadata = fs::metadata(db_path)
        .with_context(|| format!("failed to stat orderk DB: {}", db_path.display()))?;
    if !metadata.is_file() {
        return Err(anyhow!("capsule export requires a SQLite DB file"));
    }

    let mut files = Vec::new();
    files.push(artifact_file("main", db_path)?);
    for (role, path) in sqlite_sidecar_paths(db_path) {
        if path.exists() {
            files.push(artifact_file(role, &path)?);
        }
    }
    let size_bytes = files.iter().map(|file| file.size_bytes).sum();
    let sha256 = aggregate_artifact_hash(&files);
    Ok(CapsuleArtifact {
        kind: CAPSULE_ARTIFACT_KIND.to_string(),
        db: db_path.to_string_lossy().to_string(),
        size_bytes,
        sha256,
        files,
    })
}

fn artifact_file(role: &str, path: &Path) -> Result<CapsuleArtifactFile> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to stat artifact file: {}", path.display()))?;
    if !metadata.is_file() {
        return Err(anyhow!(
            "capsule artifact path is not a file: {}",
            path.display()
        ));
    }
    Ok(CapsuleArtifactFile {
        role: role.to_string(),
        path: path.to_string_lossy().to_string(),
        size_bytes: metadata.len(),
        sha256: sha256_file(path)?,
    })
}

fn aggregate_artifact_hash(files: &[CapsuleArtifactFile]) -> String {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.role.as_bytes());
        hasher.update([0]);
        hasher.update(file.size_bytes.to_le_bytes());
        hasher.update([0]);
        hasher.update(file.sha256.as_bytes());
        hasher.update([0xff]);
    }
    hex::encode(hasher.finalize())
}

fn read_capsule_manifest_bounded(manifest_path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::metadata(manifest_path).with_context(|| {
        format!(
            "failed to stat capsule manifest: {}",
            manifest_path.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(anyhow!(
            "capsule manifest path is not a file: {}",
            manifest_path.display()
        ));
    }
    if metadata.len() > MAX_CAPSULE_MANIFEST_BYTES {
        return Err(anyhow!(
            "capsule manifest too large: {} bytes exceeds {} byte limit",
            metadata.len(),
            MAX_CAPSULE_MANIFEST_BYTES
        ));
    }
    let mut file = fs::File::open(manifest_path).with_context(|| {
        format!(
            "failed to read capsule manifest: {}",
            manifest_path.display()
        )
    })?;
    let mut raw = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(MAX_CAPSULE_MANIFEST_BYTES + 1)
        .read_to_end(&mut raw)
        .with_context(|| {
            format!(
                "failed to read capsule manifest: {}",
                manifest_path.display()
            )
        })?;
    if raw.len() as u64 > MAX_CAPSULE_MANIFEST_BYTES {
        return Err(anyhow!(
            "capsule manifest too large: read exceeds {} byte limit",
            MAX_CAPSULE_MANIFEST_BYTES
        ));
    }
    Ok(raw)
}

fn validate_capsule_output_path(
    db_path: &Path,
    vault_path: Option<&Path>,
    out_path: &Path,
) -> Result<()> {
    if fs::symlink_metadata(out_path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(anyhow!(
            "capsule output must not be a symlink: {}",
            out_path.display()
        ));
    }
    let out = normalize_path(canonical_or_intended(out_path)?);
    let db_identity = normalize_path(canonical_or_intended(db_path)?);
    let mut forbidden = vec![db_identity.clone()];
    for (_, sidecar) in sqlite_sidecar_paths(&db_identity) {
        forbidden.push(normalize_path(canonical_or_intended(&sidecar)?));
    }
    if forbidden.iter().any(|path| path == &out)
        || forbidden
            .iter()
            .filter(|path| path.exists() && out_path.exists())
            .any(|path| same_file(path, out_path).unwrap_or(false))
    {
        return Err(anyhow!(
            "capsule output must not overwrite the SQLite DB or its sidecars: {}",
            out_path.display()
        ));
    }
    let mut vaults_to_check = Vec::new();
    if let Some(vault) = vault_path {
        vaults_to_check.push(vault.to_path_buf());
    }
    if let Some(inferred) = infer_vault_from_standard_db_path(&db_identity) {
        vaults_to_check.push(inferred);
    }
    for vault in vaults_to_check {
        let vault =
            normalize_path(fs::canonicalize(&vault).with_context(|| {
                format!("failed to canonicalize vault path: {}", vault.display())
            })?);
        if out.starts_with(vault) {
            return Err(anyhow!(
                "capsule output must not write inside the vault: {}",
                out_path.display()
            ));
        }
    }
    if out_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown"))
        .unwrap_or(false)
    {
        return Err(anyhow!(
            "capsule output must not be a Markdown note: {}",
            out_path.display()
        ));
    }
    reject_symlink_ancestors(out_path)?;
    Ok(())
}

fn reject_symlink_ancestors(path: &Path) -> Result<()> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let parent = absolute
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("/"));
    let mut current = PathBuf::new();
    for component in parent.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                current.pop();
            }
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(component.as_os_str()),
            Component::Normal(part) => {
                current.push(part);
                match fs::symlink_metadata(&current) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err(anyhow!(
                            "capsule output parent must not contain a symlink: {}",
                            current.display()
                        ));
                    }
                    Ok(metadata) => {
                        if !metadata.is_dir() {
                            return Err(anyhow!(
                                "capsule output parent is not a directory: {}",
                                current.display()
                            ));
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                        return Err(anyhow!(
                            "capsule output parent directory must already exist: {}",
                            current.display()
                        ));
                    }
                    Err(err) => {
                        return Err(err).with_context(|| {
                            format!(
                                "failed to inspect capsule output parent: {}",
                                current.display()
                            )
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

fn infer_vault_from_standard_db_path(db_path: &Path) -> Option<PathBuf> {
    let file = db_path.file_name()?.to_str()?;
    if file != "orderk.sqlite" {
        return None;
    }
    let orderk_dir = db_path.parent()?;
    if orderk_dir.file_name()?.to_str()? != "orderk" {
        return None;
    }
    let obsidian_dir = orderk_dir.parent()?;
    if obsidian_dir.file_name()?.to_str()? != ".obsidian" {
        return None;
    }
    obsidian_dir.parent().map(Path::to_path_buf)
}

fn write_file_without_following_symlink(out_path: &Path, payload: &[u8]) -> Result<()> {
    let parent = out_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = out_path
        .file_name()
        .ok_or_else(|| anyhow!("path has no file name: {}", out_path.display()))?;
    let tmp_path = parent.join(format!(
        ".{}.tmp-{}-{}",
        file_name.to_string_lossy(),
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .with_context(|| {
                format!("failed to create capsule temp file: {}", tmp_path.display())
            })?;
        file.write_all(payload).with_context(|| {
            format!("failed to write capsule temp file: {}", tmp_path.display())
        })?;
        file.sync_all()
            .with_context(|| format!("failed to sync capsule temp file: {}", tmp_path.display()))?;
    }
    if fs::symlink_metadata(out_path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        let _ = fs::remove_file(&tmp_path);
        return Err(anyhow!(
            "capsule output must not be a symlink: {}",
            out_path.display()
        ));
    }
    fs::rename(&tmp_path, out_path).with_context(|| {
        let _ = fs::remove_file(&tmp_path);
        format!("failed to write capsule manifest: {}", out_path.display())
    })?;
    Ok(())
}

fn canonical_or_intended(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path)
            .with_context(|| format!("failed to canonicalize path: {}", path.display()));
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("path has no file name: {}", path.display()))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if parent.exists() {
        Ok(fs::canonicalize(parent)
            .with_context(|| format!("failed to canonicalize path parent: {}", parent.display()))?
            .join(file_name))
    } else if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

fn same_file(left: &Path, right: &Path) -> Result<bool> {
    let left = fs::metadata(left)?;
    let right = fs::metadata(right)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(left.dev() == right.dev() && left.ino() == right.ino())
    }
    #[cfg(not(unix))]
    {
        Ok(false)
    }
}

fn sqlite_sidecar_paths(db_path: &Path) -> Vec<(&'static str, PathBuf)> {
    vec![
        ("wal", sqlite_sidecar_path(db_path, "-wal")),
        ("shm", sqlite_sidecar_path(db_path, "-shm")),
    ]
}

fn sqlite_sidecar_path(db_path: &Path, suffix: &str) -> PathBuf {
    let mut raw: OsString = db_path.as_os_str().to_os_string();
    raw.push(suffix);
    PathBuf::from(raw)
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open file for checksum: {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.write_all(&buf[..read])?;
    }
    Ok(hex::encode(hasher.finalize()))
}
