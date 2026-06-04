# orderk V2 Sword Spirit PRD Receipt

- Timestamp: 2026-06-04T01:39:20+08:00
- Primary artifact: `docs/charters/orderk-v2-sword-spirit-prd.md`
- Index artifact: `docs/charters/README.md`
- Task intent: Capture the orderk V2 concept as a detailed PRD/architecture document and land the first Sword Spirit MVP sidecar proposal loop.

## Effect

The PRD defines orderk V2 as a Markdown-first, Rust-driven intelligent retrieval tool that keeps V1's fast passive retrieval blade, adds one governed Sword Spirit digest loop, and uses wiki/graph growth to let knowledge stock `x` thicken when new raw material `y` arrives. It explicitly distinguishes orderk V2 from Hindsight: Hindsight is a full main-brain system with reflect throughout the lifecycle; orderk V2 keeps reflect out of the default query path and concentrates growth in one Sword Spirit digest point.

## Key decisions recorded

- Core definition: from passive retrieval blade to sword-spirit knowledge tool.
- Boundary: no Hindsight clone, no default query-time reflect, no raw mutation by LLM, no HelixNotes/Obsidian lock-in.
- Language/stack: Rust core first; Tauri/Svelte cockpit later; SQLite/FTS/vector first, Tantivy only after benchmark gate.
- HelixNotes: useful UI/vault/import/graph reference; not the orderk core foundation due to AGPL, youth, human-note-app focus, wikilink graph, and lack of vector/reranker/score trace.
- Governance: one Sword Spirit digest loop; schema-first; search JSON contract; V1 baseline; V1→V2 migration states; rollback/audit; quantified fast/accurate/sharp/stable gates.
- MVP implementation: `orderk sword run/status` writes only `.orderk/sword_spirit/runs/<run_id>/` sidecar artifacts, uses collision-resistant run IDs, rejects custom `--out-dir`, records Hindsight-aligned model metadata (`anthropic` / `MiniMax-M3`) without reading or logging API keys, and keeps source Markdown unchanged.

## Verification

- Local structure check: Markdown code fences balanced; key sections present (`10.1 最小数据契约`, `12.1 Search Result Contract`, `V1 审计基线`, risk section, final definition).
- Four-model review round 1: GPT / SF DeepSeek V4 Pro / MiMo / MiniMax returned REQUEST_CHANGES except MiMo PASS; blockers were incorporated.
- Four-model review round 2: GPT / SF DeepSeek V4 Pro / MiMo / MiniMax all returned PASS; Blocking gaps were None.
- GPT terminal review after non-blocking patch incorporation: PASS; Blocking gaps None.
- Source index updated: `docs/charters/README.md` links the new PRD.
- V1 GitHub Release verification: `gh release view v0.1.12` showed asset `orderk-v0.1.12-linux-x64`; `gh release download v0.1.12 --pattern orderk-v0.1.12-linux-x64` succeeded; local SHA256 matched GitHub asset digest `sha256:94193ddcc35057a77fae206188131d892a8aee61392617c1986ae5f810289a01`; downloaded binary reported `0.1.12` via `--version`.
- MVP local gates: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-features -- -D warnings`, `cargo test --workspace --all-features` (`orderk-cli` 14 tests, `orderk-core` 66 tests), and `python3 scripts/release_gate.py` all passed.
- MVP fixture smoke: `orderk sword run/status` on copied fixture vault generated `orderk.sword_spirit.run.v1`; proposal rows used `orderk.sword_spirit.proposal_mvp.v0`; second run ID was distinct; `--out-dir` was rejected; re-indexing the vault did not index `.orderk/sword_spirit` sidecar Markdown.
- MVP real-corpus smoke: copied `/home/agent/noteriv-vaults/obsidian-migration-test` to `/tmp/orderk-sword-real-vault`; 3713 Markdown files / 15,228,624 bytes; `orderk sword run --max-files 300 --max-proposals 80` completed in 4.58s with max RSS 14,196 KB and wrote only sidecar artifacts; subsequent mock index completed with 3713 files / 32,844 chunks and sidecar-indexed count 0; 30 sampled source Markdown hashes were unchanged.
- Subagent re-audit: two independent read-only audits returned PASS. Patch artifact `/tmp/orderk-sword-mvp-review-v2.patch` had SHA256 `597c19d30a36872720e4081c8912fbb733b4c7e7c8942e6c41af5c9f37a16b79`; audits verified forbidden naming was gone, sidecar boundary held, run IDs no longer collided, and `--out-dir` rejection/index pollution fixes worked.

## Audit evidence files

- Round 1 outputs: `/tmp/orderk_v2_audits/{gpt,sf-deepseek-v4-pro,mimo,minimax}.txt`
- Round 2 outputs: `/tmp/orderk_v2_audits_round2/{gpt,sf-deepseek-v4-pro,mimo,minimax}.txt`
- MVP patch artifact: `/tmp/orderk-sword-mvp-review-v2.patch` (`sha256=597c19d30a36872720e4081c8912fbb733b4c7e7c8942e6c41af5c9f37a16b79`)
- Release gate summary: `/tmp/orderk-release-gate-after-sword.json`
- Fixture smoke outputs: `/tmp/orderk-sword-run.json`, `/tmp/orderk-sword-run2.json`, `/tmp/orderk-sword-status.json`, `/tmp/orderk-sword-index.json`
- Real-corpus smoke outputs: `/tmp/orderk-sword-real-run.json`, `/tmp/orderk-sword-real-status.json`, `/tmp/orderk-sword-real-index.json`, `/tmp/orderk-sword-real-before.json`

## Rollback

This change now includes PRD docs plus the Sword Spirit MVP code path. Rollback options:

1. Remove `docs/charters/orderk-v2-sword-spirit-prd.md`, `docs/charters/orderk-v2-sword-spirit-prd-receipt-2026-06-04.md`, and `crates/orderk-core/src/sword_spirit.rs`.
2. Revert the Sword Spirit wiring in `.gitignore`, `crates/orderk-cli/src/main.rs`, `crates/orderk-core/src/lib.rs`, and `crates/orderk-core/src/scanner.rs`.
3. Remove the PRD row from `docs/charters/README.md`.

If this change is still uncommitted, rollback with `git restore .gitignore crates/orderk-cli/src/main.rs crates/orderk-core/src/lib.rs crates/orderk-core/src/scanner.rs docs/charters/README.md docs/charters/orderk-v2-sword-spirit-prd.md docs/charters/orderk-v2-sword-spirit-prd-receipt-2026-06-04.md crates/orderk-core/src/sword_spirit.rs`. If it has landed as a commit, rollback with `git revert <that_commit>`.

## Future potential

Use this PRD plus the MVP command `orderk sword run/status` as the P0 source of truth for: golden query design, V1 baseline, V1→V2 migration plan, Sword Spirit proposal schema, external Markdown-base adapter spikes, and cockpit scoping.
