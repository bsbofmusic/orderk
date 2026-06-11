# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.20] - 2026-06-11

### Added
- Wire `orderk jianling chat-smoke` and `self-check` into the CLI, add live Anthropic-compatible MiniMax M3 connectivity receipts, and allow Jianling to consume `ORDERK_SWORD_LLM_API_KEY_ENV` indirection without storing API key values.
- Add explicit profile-scoped Jianling LLM reflection enablement via `ORDERK_JIANLING_LLM_ENABLED_<PROFILE>` (or global `ORDERK_JIANLING_LLM_ENABLED`), with `jianling run` writing live MiniMax M3 reflection text only when the switch is enabled.
- Add Jianling self-check coverage for LLM profile configuration, global profile lock availability, and `brain/{daily,weekly,monthly}` output paths.

### Changed
- Move weekly/monthly Jianling outputs to PRD paths `brain/weekly/YYYY-MM-DD.md` and `brain/monthly/YYYY-MM-DD.md`, replace per-mode locks with a profile-wide global run lock, and make every non-empty run pass a Kanban writer/auditor/foreman harness before final Markdown is written; partial source handling remains explicit when bounded windows are exceeded.

### Verification
- `cargo fmt --check`, `cargo test -p orderk-core --test jianling_contract`, `cargo test --workspace`, and `cargo clippy --workspace -- -D warnings` passed locally. Live MiniMax M3 `chat-smoke`, live `jianling run`, and a 2026-06-01..2026-06-10 drill passed with 10 daily runs, one weekly run, one monthly run, Kanban writer/auditor/foreman final-write gates, and `validate-run` receipts.

## [0.1.19] - 2026-06-11

### Fixed
- Fix CI smoke tests by making mock-provider search explicitly use `--reranker none`, preserving the real Qwen reranker fail-closed default for normal search while allowing keyless CI/test fixtures.
- Supersede `0.1.18` after its npm tarball was already published; npm packages are immutable, so this patch release carries the CI fix instead of rewriting history.

## [0.1.18] - 2026-06-11

### Added
- Ship the first `orderk jianling` Markdown memory-compiler slice with deterministic digest generation, `run/status/doctor/enable/disable/validate-*` CLI subcommands, managed systemd-user unit/timer templates, and `.orderk/jianling/` receipt/evidence/watermark sidecars.
- Add contract tests for dry-run safety, generated Markdown source anchors, receipt/lock cleanup, validator rejection of unsourced generated notes, scheduler file generation, symlink escape blocking, tampered run validation, and managed-unit deletion safety.

### Changed
- Document the 0.1.18 Jianling state as a conservative built-in sidecar: raw transcripts stay untouched, query-time search remains read-only, and pre-0.1.20 LLM reflection is not silently faked before provider gates.

### Fixed
- Supersede the known `v0.1.17` release-line CI caveat with this patch release gate.
- Harden Jianling writes/validation against symlink escape, tampered evidence packs, stale file-op hashes, and unsafe scheduler sidecar deletion.

## [0.1.17] - 2026-06-10

### Added
- Add full-vault-smart retrieval signals: source-tier inference for transcripts/reports/system snapshots/wiki/brain/raw evidence, event-time inference from existing fields/paths, and intent-aware candidate lanes for historical/config/concept queries.
- Extend the frozen eval gate from 11 to 14 cases with historical transcript, system snapshot, and report fixtures; strict baseline remains top1/hit@k/recall/NDCG/MRR all 1.0.

### Changed
- Make default search reranking a real SiliconFlow `Qwen/Qwen3-Reranker-4B` model call. Routing now reports `metadata_intent+qwen3-reranker-4b`; results carry `qwen_reranker` evidence; CLI/MCP keep `--reranker none` / `reranker: "none"` as the only explicit test/migration escape hatch and reject legacy `--no-rerank` / `rerank: false` disable paths.

### Verification
- `cargo fmt --check` + `git diff --check` exit 0; `cargo test --workspace --all-targets` exit 0 (33 CLI tests + 114 core tests + 6/4/2 integration tests); `cargo clippy --workspace --all-targets -- -D warnings` exit 0; `python3 scripts/eval.py` exit 0 with 14/14 top1, recall@k=1.0, NDCG=1.0, MRR=1.0, mean_took_ms≈109.8; real Qwen reranker smoke exit 0 with `routing.reranker_mode=metadata_intent+qwen3-reranker-4b` and `qwen_reranker` evidence.

## [0.1.16] - 2026-06-09

### Added
- Add the frozen orderk search evaluation gate with qrels/fixture hashes, mandatory reranker evidence checks, and per-case regression protection.

### Changed
- Make lexical reranking mandatory by default for CLI and MCP search while keeping explicit `--reranker none` as a test/migration escape hatch.
- Decouple candidate depths from display limit, fuse keyword/vector/route candidates with bounded RRF, and add same-file MMR diversity before truncation.

### Fixed
- Improve route recall for Chinese/mixed queries by scanning semantic query terms against path/title/heading evidence, enforcing a global route candidate cap, and scoring multi-term route matches above single-brand matches.

## [0.1.15] - 2026-06-05

### Added
- Complete the V2 full-PRD local release-ready evidence loop with deterministic gates, proposal governance, graph/digest hardening, evidence-only reasoning, read-only adapters, and full-vault active Sword Spirit verification.

### Fixed
- Harden release evidence boundaries so blocked active runs, sample benches, and full-vault active gates are reported separately instead of being conflated.
- Reuse the stored embedding profile for existing index databases before ambient embedding environment defaults, while still allowing explicit CLI profile flags to override it.

## [0.1.14] - 2026-06-04

### Added
- Add a full-vault active Sword Spirit gate that copies the real 3,713 Markdown vault, runs live HS-aligned embedding/reranker/MiniMax typed decisions, verifies sidecar artifacts, and fails on raw Markdown mutation or fallback.

### Fixed
- Parse MiniMax Anthropic-compatible single-decision JSON and thinking-plus-text responses, while disabling thinking in the request contract for typed decisions.
- Preserve base search ordering when Sword sidecar evidence does not apply, preventing non-Sword top1 regressions from enlarged candidate windows.
- Respect typed LLM rejects as valid decisions instead of manufacturing fallback proposals when the LLM path is parseable.
- Limit large-corpus Sword candidate generation with indexed token/tag/title pools so full-vault active digest completes within a bounded race gate.

## [0.1.13] - 2026-06-04

### Added
- Add Sword Spirit active race gates with RRF source-rank traces, evidence-gated typed-edge decisions, rejected-decision sidecars, and query-time LLM=0 search metadata.
- Add real-vault Sword Spirit benchmark harness with representative sampling, base-vs-Sword deltas, fallback/rejection sidecar counts, and resource summaries.

### Fixed
- Keep Sword Spirit sidecar boosts observational and file-diverse so they cannot demote a stronger base top hit.
- Block cross-scope same-name auto-link candidates and reject out-of-vocabulary LLM relations instead of normalizing them silently.

## [0.1.12] - 2026-06-01

### Added
- Add a 10-query deterministic eval MVP with fixture-backed golden cases and strict release baseline.

### Fixed
- Keep eval fixtures and baselines aligned so missing fixture files fail the release gate.

### CI
- Run release-gate clippy with `--all-targets --all-features`.

## [0.1.11] - 2026-05-31

### Added
- Add OpenAI-compatible embedding provider plumbing for `openai`, `openai-compatible`, and `generic` providers.

### Fixed
- Keep orderk embedding credentials scoped to `ORDERK_*` variables and reject Hermes/SF or bare legacy SiliconFlow keys.
- Inherit embedding profile for CLI search.

### Changed
- Export the reusable `OpenAiCompatibleEmbeddingProvider` and config from `orderk-core`.

## [0.1.10] - 2026-05-28

### Fixed
- Resolve package vendor binary from npx symlink (npm)

### Changed
- Consolidate npm package publishing

### CI
- Fix npm publish version output
- Add npm trusted publishing workflow
- Pin Rust toolchain and satisfy clippy

### Testing
- Stabilize live orderk health query

## [0.1.9] - 2026-05-20

### Added
- Unattended Rust search optimizer
- Manual optimizer tuning and search prompt
- Polish optimizer tuning UX
- Absorb retrieval workflow refinements
- Continuous retrieval quality monitoring section to MAINTAIN.md
- Honest benchmark battle reports

### Changed
- Harden orderk health and resource gates
- Harden product gates — pycache cleanup, runtime baseline v2, live eval queries

### Fixed
- Pass dimensions param for Qwen3 series compatibility (embedding)

### Documentation
- Align README, docs, benchmarks, audit report with 5 retrieval workflow controls
- Cross-project absorption audit — orderk vs 2026 retrieval landscape

## [0.1.8] - 2026-05-18

### Added
- Complete temporal quality recall phases

## [0.1.7] - 2026-05-18

### Added
- Eval evidence URI and explain trace
- Compact recall get flow
- Verifiable capsule manifests
- Explicit retrieval depth evidence

### Documentation
- Clarify orderk value proposition

## [0.1.6] - 2026-05-13

### Added
- Obsidian link expansion recall
- Supermemory-inspired retrieval evidence
- Metadata/frontmatter-aware rerank

### Changed
- Decouple runtime binary from build artifacts

## [0.1.5] - 2026-05-12

### Added
- Structured filters and metadata

## [0.1.4] - 2026-05-12

### Added
- Orderk maintenance release gate

### Documentation
- Sharpen orderk positioning

## [0.1.3] - 2026-05-11

### Misc
- Release orderk 0.1.3

## [0.1.2] - 2026-05-11

### Fixed
- Harden orderk release and reindex safety

## [0.1.1] - 2026-05-11

### Fixed
- Make orderk npm install resolve packaged binary

## [0.1.0] - 2026-05-11

### Added
- Initial orderk release
- Ship orderk npm packages and release flow

[Keep a Changelog]: https://keepachangelog.com/
[Unreleased]: https://github.com/bsbofmusic/orderk/compare/v0.1.20...HEAD
[0.1.20]: https://github.com/bsbofmusic/orderk/compare/v0.1.19...v0.1.20
[0.1.19]: https://github.com/bsbofmusic/orderk/compare/v0.1.18...v0.1.19
[0.1.18]: https://github.com/bsbofmusic/orderk/compare/v0.1.17...v0.1.18
[0.1.17]: https://github.com/bsbofmusic/orderk/compare/v0.1.16...v0.1.17
[0.1.16]: https://github.com/bsbofmusic/orderk/compare/v0.1.15...v0.1.16
[0.1.15]: https://github.com/bsbofmusic/orderk/compare/v0.1.14...v0.1.15
[0.1.14]: https://github.com/bsbofmusic/orderk/compare/v0.1.13...v0.1.14
[0.1.13]: https://github.com/bsbofmusic/orderk/compare/v0.1.12...v0.1.13
[0.1.12]: https://github.com/bsbofmusic/orderk/compare/v0.1.11...v0.1.12
[0.1.11]: https://github.com/bsbofmusic/orderk/compare/v0.1.10...v0.1.11
[0.1.10]: https://github.com/bsbofmusic/orderk/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/bsbofmusic/orderk/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/bsbofmusic/orderk/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/bsbofmusic/orderk/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/bsbofmusic/orderk/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/bsbofmusic/orderk/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/bsbofmusic/orderk/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/bsbofmusic/orderk/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/bsbofmusic/orderk/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/bsbofmusic/orderk/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/bsbofmusic/orderk/releases/tag/v0.1.0
