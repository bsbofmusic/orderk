# orderk Memvid capsule charter

Date: 2026-05-17
Branch: `steal/memvid-capsule-orderk-20260517-101313`
Rollback tag: `pre-steal-orderk-memvid-capsule-20260517-101313`
Base: `6e1774a`

## Decision

Proceed with a tiny Memvid-inspired **capsule manifest / inspect** slice for orderk.

Do **not** port Memvid's `.mv2` storage, video framing, encryption capsule, WAL recovery, doctor repair, timeline DB, or multimodal stack. The useful lesson for orderk is the product/engineering shape: a portable, self-describing, verifiable memory/index artifact that clearly binds payload, profile, schema, and checksums.

## Residual Graphiti/OpenMemory decision

Two residual audits concluded `MOVE_ON`:

- Graphiti: prior orderk `--retrieval-depth 1` captured the highest-value compatible slice: explicit traversal depth, provenance/evidence fields, bounded link expansion. Remaining Graphiti features (temporal facts, graph DB lifecycle, episode mutation) violate orderk's read-only Obsidian retrieval-blade boundary or are too heavy.
- OpenMemory: prior orderk evidence/scoring work captured the compatible slice: explainable scoring, bounded boosts, agent-facing retrieval surface. Remaining OpenMemory memory CRUD/consolidation/daemon/user app pieces are outside boundary.

## Memvid source evidence

Audited source repo `/tmp/orderk-steal-residual-memvid/memvid` at `178e277`.

Useful atoms:

1. README lines 56-80: Memvid packages data, embeddings, search structure, and metadata into one portable file; Smart Frames are immutable units with timestamps/checksums and append-only semantics.
2. `src/types/frame.rs` lines 169-231: each frame carries timestamp, payload offset/length, checksum, uri/title, tags/metadata, chunk info, source SHA-256/path.
3. `src/types/manifest.rs` lines 136-202 and 235-266: manifest/segment catalog includes bounded deserialization, offsets/lengths/checksums, segment stats, span.
4. `src/types/embedding_identity.rs` lines 8-23: embedding identity persists provider/model/dimension/normalized, not just vector dimension.
5. `src/io/header.rs` lines 26-145: fixed header validates magic/version/offset/checksum metadata before trusting payload.
6. `src/io/wal.rs` lines 10-205: WAL records carry seq/len/checksum and enforce read-only behavior when opened read-only.
7. Tests `model_consistency.rs`, `replay_integrity.rs`, `crash_recovery.rs`, `doctor_recovery.rs`: profile mismatch, corruption, replay integrity, and recovery are first-class test targets.

## Target boundary

orderk remains:

- local Rust CLI/MCP retrieval blade;
- Obsidian Markdown source of truth;
- SQLite sidecar index/cache;
- read-only with respect to vault notes;
- no daemon, no chat, no note generation, no automatic memory mutation.

## Absorb / Adapt / Reject

| Category | Memvid atom | orderk slice |
|---|---|---|
| Absorb | Self-describing portable artifact | `orderk capsule export --db <sqlite> --out <json>` writes a compact manifest JSON, not a new binary DB |
| Absorb | Payload checksum + size | manifest records combined SQLite main DB + existing `-wal` / `-shm` sidecar byte size and aggregate SHA-256, plus per-file checksums; inspect recomputes and verifies |
| Absorb | Model/profile binding | manifest records schema version, provider, model, dim, vector backend, notes/chunks/embeddings |
| Absorb | Source/capsule stats | manifest records optional vault path plus source counts; no note content |
| Adapt | Fixed header/magic/version | JSON has `schema_version: orderk.capsule.v1`, `artifact.kind: orderk.sqlite_index` |
| Adapt | Doctor/report style | `orderk capsule inspect --file manifest --db <sqlite>` returns structured `checks`, not repair |
| Reject | `.mv2` binary container / Smart Frame storage | too large; orderk already has SQLite sidecar |
| Reject | WAL/recovery/doctor repair | no repair/import; export/inspect only verifies existing SQLite main+sidecar payload |
| Reject | encryption/multimodal/cloud API | outside read-only retrieval blade |

## Minimal implementation

New CLI subcommand:

```bash
orderk capsule export --db /path/orderk.sqlite --out /path/orderk.capsule.json [--vault /path/vault]
orderk capsule inspect --file /path/orderk.capsule.json --db /path/orderk.sqlite
```

Core API:

- `export_capsule_manifest(db_path, vault_path) -> CapsuleManifest`
- `write_capsule_manifest(db_path, vault_path, out_path) -> CapsuleManifest`
  - rejects `out_path` equal to the DB or SQLite sidecars;
  - rejects hardlinks/symlinks and `..` traversal to DB, SQLite sidecars, or vault paths;
  - rejects output paths inside the supplied vault or standard inferred vault (`.obsidian/orderk/orderk.sqlite` layout);
  - rejects Markdown file extensions case-insensitively;
- `inspect_capsule_manifest(file_path, db_path) -> CapsuleInspection`
  - refuses manifests over 1 MiB before JSON parsing;
  - validates manifest-internal artifact file hash/size consistency before DB comparison.

Manifest shape:

```json
{
  "schema_version": "orderk.capsule.v1",
  "artifact": {"kind": "orderk.sqlite_index", "db": "...", "size_bytes": 123, "sha256": "...", "files": [{"role":"main", "path":"...", "size_bytes":123, "sha256":"..."}]},
  "profile": {"schema_version": "4", "embedding_provider": "mock", "embedding_model": "mock", "embedding_dim": 8, "vector_backend": "exact"},
  "stats": {"notes": 2, "chunks": 2, "embeddings": 2, "fts_enabled": true, "vector_enabled": true},
  "source": {"vault": "..."},
  "created_at": "..."
}
```

Inspect shape:

```json
{
  "schema_version": "orderk.capsule_inspection.v1",
  "ok": true,
  "checks": [
    {"component":"manifest_schema", "ok":true, ...},
    {"component":"db_checksum", "ok":true, ...},
    {"component":"profile", "ok":true, ...},
    {"component":"stats", "ok":true, ...}
  ]
}
```

## Non-goals

- No copying DB into the manifest in this slice.
- No import/restore yet.
- No compressed bundle/tar format yet.
- No writing vault Markdown.
- No automatic repair of corrupted DB or manifest.
- No remote sync or cloud artifact registry.

## Verification gates

TDD RED/GREEN:

1. Core test: exporting a manifest after indexing a sample vault records aggregate checksum/profile/stats and inspect passes.
2. Core test: mutating/corrupting DB after export makes inspect fail `db_checksum`.
3. Core test: WAL sidecar payload is included in artifact files and aggregate checksum.
4. Core test: real DB `settings.schema_version` drift fails profile check.
5. Core test: export rejects DB/sidecar overwrite, hardlink/symlink outputs, traversal into DB/vault, inferred standard vault output, and Markdown extensions case-insensitively.
6. Core test: inspect rejects manifests over 1 MiB before JSON parsing.
7. Core test: inspect fails manifest-internal artifact file tampering even if top-level checksum field is left unchanged.
8. CLI contract test: `orderk capsule export/inspect` emits JSON and does not add MCP write tools.


Release gates:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo build --workspace --all-features --release`
- `python3 scripts/release_gate.py`
- stable artifact install + smoke
- independent review
- push to remote default branch and verify remote contains HEAD
- clean `/tmp/orderk-steal-residual-memvid`, review bundles, temporary tool dirs after stable smoke

## Rollback

- Git rollback: `git reset --hard 6e1774a` or `git switch main && git reset --hard origin/main` before push.
- Rollback tag: `pre-steal-orderk-memvid-capsule-20260517-101313`.
- Stable binary rollback: before install, copy `/home/agent/.local/bin/orderk` to timestamped `.bak-*`.

## Go criteria

Proceed only if implementation remains a small JSON manifest/inspect layer, uses no new heavy dependency, does not write vault notes, does not expose write-capable MCP tools, and passes stable binary smoke.
