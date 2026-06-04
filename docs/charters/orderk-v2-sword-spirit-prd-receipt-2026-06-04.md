# orderk V2 Sword Spirit PRD Receipt

- Timestamp: 2026-06-04T01:39:20+08:00
- Primary artifact: `docs/charters/orderk-v2-sword-spirit-prd.md`
- Index artifact: `docs/charters/README.md`
- Task intent: Capture the orderk V2 concept as a detailed PRD/architecture document and land the first Sword Spirit MVP sidecar proposal loop, then put the active Sword Spirit path on a Hindsight comparison track.

## Effect

The PRD defines orderk V2 as a Markdown-first, Rust-driven intelligent retrieval tool that keeps V1's fast passive retrieval blade, adds one governed Sword Spirit digest loop, and uses wiki/graph growth to let knowledge stock `x` thicken when new raw material `y` arrives. It explicitly distinguishes orderk V2 from Hindsight: Hindsight is a full main-brain system with reflect throughout the lifecycle; orderk V2 keeps reflect out of the default query path and concentrates growth in one Sword Spirit digest point.

## Key decisions recorded

- Core definition: from passive retrieval blade to sword-spirit knowledge tool.
- Boundary: no Hindsight clone, no default query-time reflect, no raw mutation by LLM, no HelixNotes/Obsidian lock-in.
- Language/stack: Rust core first; Tauri/Svelte cockpit later; SQLite/FTS/vector first, Tantivy only after benchmark gate.
- HelixNotes: useful UI/vault/import/graph reference; not the orderk core foundation due to AGPL, youth, human-note-app focus, wikilink graph, and lack of vector/reranker/score trace.
- Governance: one Sword Spirit digest loop; schema-first; search JSON contract; V1 baseline; V1→V2 migration states; rollback/audit; quantified fast/accurate/sharp/stable gates.
- MVP implementation: `orderk sword run/status` writes only `.orderk/sword_spirit/runs/<run_id>/` sidecar artifacts, uses collision-resistant run IDs, rejects custom `--out-dir`, records Hindsight-aligned model metadata (`anthropic` / `MiniMax-M3`) without reading or logging API keys in heuristic mode, and keeps source Markdown unchanged.
- Active track correction: `orderk sword run --thinking active` now invokes the Hindsight-aligned provider stack across the digest loop: SiliconFlow `Qwen/Qwen3-Embedding-4B` at 1024 dimensions creates embedding neighbors, SiliconFlow `Qwen/Qwen3-Reranker-4B` reranks them, and MiniMax M3 is called through Anthropic-compatible messages to keep typed semantic edges. Active proposals are normalized to the PRD P3 edge vocabulary: `supports`, `refines`, `contradicts`, `replaces`, `depends_on`, `part_of`.
- Search integration: `orderk sword search` loads the latest Sword Spirit sidecar proposals and applies a small gated graph/sidecar boost only when query terms overlap the proposal evidence/rationale, preventing generic sidecar edges from dragging down unrelated fast searches.

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
- Active Sword Spirit smoke: `/tmp/orderk-sword-active-smoke` used 3 Markdown notes; `orderk sword run --thinking active --max-files 10 --max-proposals 6` completed with `llm_invocation=called`, `reranker_invocation=called`, 4 candidates, 4 reranked candidates, 1 MiniMax M3 call, 2 accepted semantic proposals, and max RSS about 13.6 MB.
- Hindsight comparison track: `scripts/sword_hs_bench.py` creates an isolated temp Hindsight bank, retains the same 4 benchmark Markdown docs, runs identical golden queries through orderk base search, `orderk sword search`, and Hindsight recall, then deletes the temp bank. First run exposed a regression: ungated sidecar boost reduced Sword top1 from 4/4 to 2/4. After tuning and adding embedding neighbors to active digest, the latest run at `/tmp/orderk-sword-hs-bench/summary.json` produced orderk base top1 4/4, Sword top1 4/4, Hindsight recall top1 4/4. Sword active digest called Qwen3 embedding (`embedded_count=4`), Qwen3 reranker (`reranked_count=12`), and MiniMax M3 (`llm_calls=1`); MiniMax M3 returned an Anthropic-compatible response with `thinking` but no parseable `text`, so the run correctly recorded `llm_invocation=called_unparseable_fallback` and persisted four review-only embedding+reranker proposals instead of failing the sidecar. Latest resource points: Sword active digest max RSS 13,656 KB / 59.00s wall; orderk Qwen3 index max RSS 17,180 KB / 3.18s wall; Hindsight temp-bank retain 59.326s; reflect probe 36.959s; temp bank delete status 200. This is a small MVP fixture, not the PRD's required 50-query gate.
- MVP completion status: the engineering MVP is complete for the scoped sidecar/digest/search path: active Sword Spirit creates sidecar artifacts, uses live HS-aligned embedding/reranker/LLM calls, preserves raw Markdown, rejects custom sidecar output dirs, avoids query-time reflection, gates sidecar search boosts, and passes the release gate. It is not the full V2 PRD: robust MiniMax typed-decision parsing/non-thinking response mode, real 3,713-file active digest production benchmark, cockpit/UI, adapter matrix for external Markdown bases, and multi-run governance/approval workflow remain future PRD work.
- Real-battle boundary update: a full-vault remote embedding attempt against `/home/agent/obsidian-vault` was stopped because it exceeded the lightweight-engineering threshold before producing a summary; evidence is saved at `/tmp/orderk-sword-full-vault-aborted-evidence.json` (`files_count=1258`, `chunks_count=8570`, `chunk_embeddings_count=8570`, DB bytes `95,617,024`). This is a degraded/too-heavy signal, not a full-vault pass.
- Representative 50-doc sample bench: `scripts/sword_real_vault_bench.py` reads the live 3,713-md source vault, copies a deterministic 50-document representative sample into `/tmp/orderk-sword-real-vault-bench/sample-vault`, indexes only that sample vault, and runs 50 deterministic real-note queries. Latest output `/tmp/orderk-sword-real-vault-bench/summary.json`: source vault `3713` Markdown files / `14.52 MB`; sample `50` Markdown files / about `0.215 MB`; orderk base top1 `34/50`, hit@3 `37/50`, hit@10 `48/50`, MRR `0.7444`; orderk Sword top1 `33/50`, hit@3 `42/50`, hit@10 `49/50`, MRR `0.7663`; active digest `embedded_count=50`, `reranked_count=24`, `llm_calls=2`, `accepted_count=5`, `active_neighbors=24`, `active_proposals=5`. Scope wording: this supports a lightweight representative-sample pass with MRR/near-neighbor improvement and a top1 regression of one query; it does not support a 3,713-md full-vault production pass.
- Sword search guard regression: the 50-query sample bench exposed that ungated sidecar boost plus chunk-level truncation could demote base hits by flooding topN with multiple chunks from a few files. A TDD regression test (`sword_sidecar_boosts_must_not_demote_base_top_hit_or_collapse_file_diversity`) now locks the fix: sidecar boosts are anchored to base top1 and final Sword results apply file-level diversity before truncation.
- Subagent re-audit: earlier MVP two-agent audit returned PASS; the later PRD/bench audit returned REQUEST_CHANGES until this receipt and PRD recorded the real-battle boundary. Final narrow read-only audit returned PASS for search guard, sample-bench boundary, and docs boundary; evidence is `/tmp/orderk-sword-real-battle-final-narrow-audit.txt` (`sha256=7f10b47f12a5345d197087628c6c8814c1664e1049e5f360dffdadefadcb3deb`).

## Audit evidence files

- Round 1 outputs: `/tmp/orderk_v2_audits/{gpt,sf-deepseek-v4-pro,mimo,minimax}.txt`
- Round 2 outputs: `/tmp/orderk_v2_audits_round2/{gpt,sf-deepseek-v4-pro,mimo,minimax}.txt`
- MVP patch artifact: `/tmp/orderk-sword-mvp-review-v2.patch` (`sha256=597c19d30a36872720e4081c8912fbb733b4c7e7c8942e6c41af5c9f37a16b79`)
- Release gate summary: `/tmp/orderk-release-gate-after-sword.json`
- Fixture smoke outputs: `/tmp/orderk-sword-run.json`, `/tmp/orderk-sword-run2.json`, `/tmp/orderk-sword-status.json`, `/tmp/orderk-sword-index.json`
- Real-corpus smoke outputs: `/tmp/orderk-sword-real-run.json`, `/tmp/orderk-sword-real-status.json`, `/tmp/orderk-sword-real-index.json`, `/tmp/orderk-sword-real-before.json`
- Active/Hindsight benchmark script: `scripts/sword_hs_bench.py`
- Active/Hindsight benchmark latest output: `/tmp/orderk-sword-hs-bench/summary.json`
- Representative real-vault sample benchmark script: `scripts/sword_real_vault_bench.py`
- Representative 50-doc real-vault sample output: `/tmp/orderk-sword-real-vault-bench/summary.json`
- Full-vault remote embedding aborted evidence: `/tmp/orderk-sword-full-vault-aborted-evidence.json`
- Post-real-battle release gate log: `/tmp/orderk-release-gate-after-sword-real-bench.txt` (`sha256=37e2635ff54029d2564c5caa3d12204602193a3f550cf7c44750e4fc6f3f7ddb`)

## Rollback

This change now includes PRD docs plus the Sword Spirit MVP code path. Rollback options:

1. Remove `docs/charters/orderk-v2-sword-spirit-prd.md`, `docs/charters/orderk-v2-sword-spirit-prd-receipt-2026-06-04.md`, and `crates/orderk-core/src/sword_spirit.rs`.
2. Revert the Sword Spirit wiring in `.gitignore`, `crates/orderk-cli/src/main.rs`, `crates/orderk-core/src/lib.rs`, `crates/orderk-core/src/scanner.rs`, and remove `scripts/sword_hs_bench.py` / `scripts/sword_real_vault_bench.py` if present.
3. Remove the PRD row from `docs/charters/README.md`.

If this change is still uncommitted, rollback with `git restore .gitignore crates/orderk-cli/src/main.rs crates/orderk-core/src/lib.rs crates/orderk-core/src/scanner.rs docs/charters/README.md docs/charters/orderk-v2-sword-spirit-prd.md docs/charters/orderk-v2-sword-spirit-prd-receipt-2026-06-04.md crates/orderk-core/src/sword_spirit.rs && rm -f scripts/sword_hs_bench.py scripts/sword_real_vault_bench.py`. If it has landed as a commit, rollback with `git revert <that_commit>`.

## Future potential

Use this PRD plus the MVP command `orderk sword run/status` as the P0 source of truth for: golden query design, V1 baseline, V1→V2 migration plan, Sword Spirit proposal schema, external Markdown-base adapter spikes, and cockpit scoping.
