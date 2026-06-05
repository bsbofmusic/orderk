use orderk_core::{scan_obsidian_adapter, AdapterScanOptions};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_vault(prefix: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "orderk-{prefix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let vault = root.join("vault");
    fs::create_dir_all(&vault).unwrap();
    vault
}

fn file_hash(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn obsidian_adapter_reads_markdown_frontmatter_wikilinks_and_attachment_metadata_only() {
    let vault = temp_vault("adapter-contract");
    fs::create_dir_all(vault.join("wiki/concepts")).unwrap();
    fs::create_dir_all(vault.join("assets")).unwrap();
    let note = vault.join("wiki/concepts/cashflow.md");
    fs::write(
        &note,
        "---\ntags: [finance, strategy]\nstatus: active\n---\n# Cashflow\nSee [[wiki/concepts/noise]] and ![[assets/chart.png]].\n#topic\n",
    )
    .unwrap();
    fs::write(
        vault.join("wiki/concepts/noise.md"),
        "# Noise\nNoise is a decoy. #strategy\n",
    )
    .unwrap();
    fs::write(vault.join("assets/chart.png"), "png-bytes").unwrap();
    let before = file_hash(&note);

    let report = scan_obsidian_adapter(&vault, &AdapterScanOptions::default()).unwrap();

    assert_eq!(file_hash(&note), before, "adapter must not mutate markdown");
    assert_eq!(report.schema_version, "orderk.adapter.obsidian.v1");
    assert_eq!(report.adapter, "obsidian");
    assert_eq!(report.write_capability, "disabled");
    assert!(report
        .files
        .iter()
        .any(|file| file.path == "wiki/concepts/cashflow.md"
            && file.title.as_deref() == Some("Cashflow")
            && file.tags.contains(&"finance".to_string())
            && file.wikilinks.contains(&"wiki/concepts/noise".to_string())));
    assert!(report
        .tags
        .iter()
        .any(|tag| tag.name == "strategy" && tag.count >= 2));
    assert!(report
        .concepts
        .iter()
        .any(|concept| concept.path == "wiki/concepts/cashflow.md"));
    assert_eq!(report.attachments.len(), 1);
    assert_eq!(report.attachments[0].path, "assets/chart.png");
    assert_eq!(
        report.attachments[0].referenced_by,
        vec!["wiki/concepts/cashflow.md"]
    );
    assert!(!report.raw_write_performed);
}

#[test]
fn obsidian_adapter_rejects_symlinked_markdown_and_stays_inside_vault() {
    let vault = temp_vault("adapter-symlink");
    fs::write(vault.join("safe.md"), "# Safe\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/passwd", vault.join("leak.md")).unwrap();

    let report = scan_obsidian_adapter(&vault, &AdapterScanOptions::default()).unwrap();

    assert!(report.files.iter().any(|file| file.path == "safe.md"));
    assert!(!report.files.iter().any(|file| file.path == "leak.md"));
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.contains("symlink")));
}
