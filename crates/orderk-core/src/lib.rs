pub mod api;
pub mod capsule;
pub mod chunker;
pub mod embedding;
pub mod filter;
pub mod health;
pub mod index;
pub mod markdown;
pub mod models;
pub mod scanner;

pub use api::{
    feedback, get_chunks, index_vault, index_vault_with_options, init, provider_from_env,
    provider_from_name, query, query_with_filter, query_with_options, status,
};
pub use capsule::{export_capsule_manifest, inspect_capsule_manifest, write_capsule_manifest};
pub use embedding::{EmbeddingProvider, MockEmbeddingProvider, SiliconFlowM3Provider};
pub use health::{classify_error_message, health_report};
pub use models::*;

#[cfg(test)]
mod capsule_contract_tests {
    use super::*;
    use rusqlite::Connection;
    use std::fs;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "orderk-{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn indexed_sample_db() -> (std::path::PathBuf, std::path::PathBuf) {
        let root = unique_temp_dir("capsule");
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(
            vault.join("alpha.md"),
            "# Alpha\nCapsule manifest proof links to [[Bravo]].\n",
        )
        .unwrap();
        fs::write(vault.join("bravo.md"), "# Bravo\nPortable proof.\n").unwrap();
        let db = root.join("orderk.sqlite");
        let provider = MockEmbeddingProvider::new(8);
        index_vault(
            &vault,
            &db,
            &provider,
            provider.dimension(),
            provider.model_id(),
            VectorBackend::Exact,
        )
        .unwrap();
        (vault, db)
    }

    #[test]
    fn capsule_export_manifest_binds_db_checksum_profile_and_stats() {
        let (vault, db) = indexed_sample_db();
        let manifest = export_capsule_manifest(&db, Some(&vault)).unwrap();

        assert_eq!(manifest.schema_version, "orderk.capsule.v1");
        assert_eq!(manifest.artifact.kind, "orderk.sqlite_index");
        assert!(manifest.artifact.size_bytes >= fs::metadata(&db).unwrap().len());
        assert_eq!(manifest.artifact.sha256.len(), 64);
        assert!(manifest
            .artifact
            .files
            .iter()
            .any(|file| file.role == "main"));
        assert_eq!(manifest.profile.schema_version, "5");
        assert_eq!(manifest.profile.embedding_provider, "mock");
        assert_eq!(manifest.profile.embedding_model, "mock-8");
        assert_eq!(manifest.profile.embedding_dim, 8);
        assert_eq!(manifest.profile.vector_backend, "exact");
        assert_eq!(manifest.stats.notes, 2);
        assert_eq!(manifest.stats.chunks, 2);
        assert_eq!(manifest.stats.embeddings, 2);
        assert_eq!(
            manifest.source.vault.as_deref(),
            Some(vault.to_string_lossy().as_ref())
        );

        let out = db.with_extension("capsule.json");
        let written = write_capsule_manifest(&db, Some(&vault), &out).unwrap();
        assert_eq!(written.artifact.kind, manifest.artifact.kind);
        assert_eq!(written.profile, manifest.profile);
        assert_eq!(written.stats, manifest.stats);

        let inspection = inspect_capsule_manifest(&out, Some(&db)).unwrap();
        assert!(inspection.ok, "inspection should pass: {inspection:?}");
        assert!(inspection
            .checks
            .iter()
            .any(|check| check.component == "db_checksum" && check.ok));
        assert!(inspection
            .checks
            .iter()
            .any(|check| check.component == "profile" && check.ok));
        assert!(inspection
            .checks
            .iter()
            .any(|check| check.component == "stats" && check.ok));
    }

    #[test]
    fn capsule_manifest_hash_includes_wal_sidecar_payload() {
        let (vault, db) = indexed_sample_db();
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn.execute(
            "INSERT INTO feedback_events(event, payload, created_at) VALUES('wal-before', '{}', 1)",
            [],
        )
        .unwrap();

        let out = db.with_extension("capsule.json");
        let manifest = write_capsule_manifest(&db, Some(&vault), &out).unwrap();
        assert!(manifest
            .artifact
            .files
            .iter()
            .any(|file| file.role == "wal"));

        conn.execute(
            "INSERT INTO feedback_events(event, payload, created_at) VALUES('wal-after', '{}', 2)",
            [],
        )
        .unwrap();

        let inspection = inspect_capsule_manifest(&out, Some(&db)).unwrap();
        assert!(!inspection.ok);
        assert!(inspection
            .checks
            .iter()
            .any(|check| check.component == "db_checksum" && !check.ok));
    }

    #[test]
    fn capsule_inspect_fails_when_db_payload_changes_after_export() {
        let (vault, db) = indexed_sample_db();
        let out = db.with_extension("capsule.json");
        write_capsule_manifest(&db, Some(&vault), &out).unwrap();

        let mut file = fs::OpenOptions::new().append(true).open(&db).unwrap();
        file.write_all(b"corrupt-tail").unwrap();
        file.flush().unwrap();

        let inspection = inspect_capsule_manifest(&out, Some(&db)).unwrap();
        assert!(!inspection.ok);
        let checksum = inspection
            .checks
            .iter()
            .find(|check| check.component == "db_checksum")
            .expect("checksum check exists");
        assert!(!checksum.ok);
    }

    #[test]
    fn capsule_inspect_detects_db_schema_profile_drift() {
        let (vault, db) = indexed_sample_db();
        let out = db.with_extension("capsule.json");
        write_capsule_manifest(&db, Some(&vault), &out).unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE settings SET value = '999' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();

        let inspection = inspect_capsule_manifest(&out, Some(&db)).unwrap();
        assert!(!inspection.ok);
        assert!(inspection
            .checks
            .iter()
            .any(|check| check.component == "profile" && !check.ok));
    }

    #[test]
    fn capsule_write_rejects_db_and_vault_markdown_outputs() {
        let (vault, db) = indexed_sample_db();
        let db_err = write_capsule_manifest(&db, Some(&vault), &db).unwrap_err();
        assert!(db_err.to_string().contains("must not overwrite"));

        let note_out = vault.join("existing-note.md");
        fs::write(&note_out, "# Existing\n").unwrap();
        let note_err = write_capsule_manifest(&db, Some(&vault), &note_out).unwrap_err();
        assert!(note_err
            .to_string()
            .contains("must not write inside the vault"));

        let markdown_out = db.with_file_name("capsule.MD");
        let markdown_err = write_capsule_manifest(&db, Some(&vault), &markdown_out).unwrap_err();
        assert!(markdown_err
            .to_string()
            .contains("must not be a Markdown note"));
    }

    #[test]
    fn capsule_write_rejects_db_hardlink_output() {
        let (vault, db) = indexed_sample_db();
        let hardlink = db.with_extension("hardlink.sqlite");
        fs::hard_link(&db, &hardlink).unwrap();
        let err = write_capsule_manifest(&db, Some(&vault), &hardlink).unwrap_err();
        assert!(err.to_string().contains("must not overwrite"));
    }

    #[test]
    fn capsule_inspect_allows_same_payload_after_db_path_changes() {
        let (vault, db) = indexed_sample_db();
        let out = db.with_extension("capsule.json");
        write_capsule_manifest(&db, Some(&vault), &out).unwrap();
        let moved = db.with_file_name("moved-orderk.sqlite");
        fs::copy(&db, &moved).unwrap();

        let inspection = inspect_capsule_manifest(&out, Some(&moved)).unwrap();
        assert!(
            inspection.ok,
            "path-only changes must not break payload verification: {inspection:?}"
        );
    }

    #[test]
    fn capsule_write_rejects_traversal_to_db_or_vault() {
        let (vault, db) = indexed_sample_db();
        let root = db.parent().unwrap();

        let standard_db = vault.join(".obsidian").join("orderk").join("orderk.sqlite");
        fs::create_dir_all(standard_db.parent().unwrap()).unwrap();
        fs::copy(&db, &standard_db).unwrap();
        let inferred_vault_err = write_capsule_manifest(
            &standard_db,
            None,
            &vault.join("capsule-outside-explicit-vault.json"),
        )
        .unwrap_err();
        assert!(inferred_vault_err
            .to_string()
            .contains("must not write inside the vault"));

        let traversed_standard_db = vault
            .join(".obsidian")
            .join("orderk")
            .join("..")
            .join("orderk")
            .join("orderk.sqlite");
        let inferred_traversal_err = write_capsule_manifest(
            &traversed_standard_db,
            None,
            &vault.join("capsule-in-vault-via-db-traversal.json"),
        )
        .unwrap_err();
        assert!(inferred_traversal_err
            .to_string()
            .contains("must not write inside the vault"));

        let db_traversal = root.join("missing-parent").join("..").join("orderk.sqlite");
        let db_err = write_capsule_manifest(&db, Some(&vault), &db_traversal).unwrap_err();
        assert!(db_err.to_string().contains("must not overwrite"));

        let vault_traversal = root
            .join("missing-parent")
            .join("..")
            .join("vault")
            .join("nested")
            .join("capsule.json");
        let vault_err = write_capsule_manifest(&db, Some(&vault), &vault_traversal).unwrap_err();
        assert!(vault_err
            .to_string()
            .contains("must not write inside the vault"));
    }

    #[cfg(unix)]
    #[test]
    fn capsule_write_rejects_broken_symlink_outputs() {
        use std::os::unix::fs::symlink;

        let (vault, db) = indexed_sample_db();
        let root = db.parent().unwrap();

        let symlink_to_vault = root.join("outside-capsule.json");
        symlink(vault.join("bad.md"), &symlink_to_vault).unwrap();
        let vault_err = write_capsule_manifest(&db, Some(&vault), &symlink_to_vault).unwrap_err();
        assert!(vault_err.to_string().contains("symlink"));
        assert!(!vault.join("bad.md").exists());

        let symlink_to_wal = root.join("outside-wal.json");
        symlink(format!("{}-wal", db.to_string_lossy()), &symlink_to_wal).unwrap();
        let wal_err = write_capsule_manifest(&db, Some(&vault), &symlink_to_wal).unwrap_err();
        assert!(wal_err.to_string().contains("symlink"));
        assert!(!std::path::PathBuf::from(format!("{}-wal", db.to_string_lossy())).exists());

        let symlink_parent = root.join("link-to-vault");
        symlink(&vault, &symlink_parent).unwrap();
        let nested = symlink_parent.join("newdir").join("capsule.json");
        let nested_err = write_capsule_manifest(&db, Some(&vault), &nested).unwrap_err();
        assert!(nested_err.to_string().contains("symlink"));
        assert!(!vault.join("newdir").exists());
    }

    #[test]
    fn capsule_inspect_detects_manifest_artifact_file_tampering() {
        let (vault, db) = indexed_sample_db();
        let out = db.with_extension("capsule.json");
        write_capsule_manifest(&db, Some(&vault), &out).unwrap();

        let mut raw: serde_json::Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
        raw["artifact"]["files"][0]["size_bytes"] = serde_json::json!(1);
        fs::write(&out, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();

        let inspection = inspect_capsule_manifest(&out, Some(&db)).unwrap();
        assert!(!inspection.ok);
        assert!(inspection
            .checks
            .iter()
            .any(|check| check.component == "artifact_manifest" && !check.ok));
    }

    #[test]
    fn capsule_inspect_rejects_oversized_manifest_before_reading() {
        let root = unique_temp_dir("capsule-large-manifest");
        fs::create_dir_all(&root).unwrap();
        let huge = root.join("huge.capsule.json");
        fs::File::create(&huge).unwrap().set_len(1_048_577).unwrap();
        let err = inspect_capsule_manifest(&huge, None).unwrap_err();
        assert!(err.to_string().contains("capsule manifest too large"));
    }
}
