# orderk temporal validity + quality charter

Date: 2026-05-18
Branch: `steal/temporal-validity-quality-20260518-072011`

## Purpose

Steal only the thin, useful slice from hippo-memory / Graphiti-style memory systems: time-aware validity and evidence-quality scoring for orderk search results.

orderk remains a read-only Obsidian retrieval blade. This change must not introduce a memory OS, daemon, cloud service, graph database, LLM extraction, autonomous consolidation, note writing, retrieval-count mutation, or any runtime memory lifecycle.

## Source lesson distilled

- hippo-memory shows that memory retrieval needs an `asOf` lens, stale/superseded exclusion, and score breakdowns that separate semantic match from time/quality signals.
- Graphiti/OpenMemory-style temporal validity windows are valuable only as read-only metadata interpretation in orderk.

## Scope

### P0 implemented in this slice

1. Read-only temporal frontmatter fields:
   - `valid_from`
   - `valid_until`
   - `updated`
   - `supersedes`
   - `superseded_by`
2. Search controls:
   - `--freshness off|balanced|recent|oldest`
   - `--as-of YYYY-MM-DD`
   - `--include-stale`
3. Result quality/validity output:
   - `validity.state`
   - `validity.age_days`
   - `validity.stale_reason`
   - `score_breakdown.freshness_boost`
   - `score_breakdown.confidence_boost`
   - `score_breakdown.status_boost`
   - `score_breakdown.evidence_count_boost`
4. Default policy:
   - exclude stale/archived/deprecated/superseded/future/expired results unless `--include-stale`
   - keep freshness boosts bounded so ordinary semantic relevance is not overwhelmed
   - allow `--as-of` to surface old evidence that was valid at the requested date

## Non-goals / hard boundaries

- No note generation or editing.
- No retrieval-count updates or strength mutation during search.
- No LLM rewrite/rerank/extraction dependency.
- No graph database or daemon.
- Public `search` and `get` keep `SQLITE_OPEN_READ_ONLY` and must not migrate databases.
- Schema migration is allowed only in index/init writable paths.

## Acceptance gates

1. RED tests exist before implementation for:
   - temporal frontmatter parsing/index persistence
   - default stale/superseded exclusion and `--include-stale`
   - `--as-of` returning the evidence valid at that historical date
   - `--freshness recent`/current cue ranking recent valid evidence above old evidence
   - score breakdown/validity fields visible in JSON models
2. Verification:
   - `cargo fmt --check`
   - `cargo clippy --workspace -- -D warnings`
   - `cargo test --workspace`
   - release gate / live smoke over the user's Obsidian orderk DB
   - independent pre-commit review
3. Release hygiene:
   - stable binary installed only after verification
   - rollback tag remains available
   - branch committed and pushed
   - temporary kitchen cleaned
   - rust-lean-tool-growth-loop reference updated with implementation outcome
