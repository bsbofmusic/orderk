# orderk temporal validity + quality P0-P3 charter

Date: 2026-05-18
Branch: `steal/temporal-quality-p0-p3-20260518-091945`
Base: `13fa722` (`feat: add temporal validity quality scoring`)
Rollback tag: `pre-steal-temporal-quality-p0-p3-20260518-091945`

## Purpose

Finish the hippo-memory / Graphiti-style temporal-quality steal as four lean orderk phases without turning orderk into a memory OS.

orderk remains a read-only Obsidian retrieval blade. This work must not introduce a daemon, cloud service, graph database, note generation, note editing, autonomous memory lifecycle, LLM extraction, retrieval-count mutation, or write-capable MCP tools.

## Source lesson distilled

- hippo-memory shows that recall should separate semantic match from time validity, freshness, status, and confidence.
- Graphiti/OpenMemory-style temporal windows are useful only as read-only metadata interpretation in orderk.
- orderk should expose enough structured evidence for agents to choose and fetch chunks, but not become a chat/memory system.

## Phase roadmap and acceptance

### P0 — Indexed temporal metadata + scoring foundation ✅ already complete

Implemented in the previous slice:

1. Read-only temporal/quality frontmatter ingestion into the sidecar index:
   - `valid_from`, `valid_until`, `updated`, `supersedes`, `superseded_by`
   - `confidence`, `status`, `source_type`
2. Search controls:
   - `--freshness off|balanced|recent|oldest`
   - `--as-of YYYY-MM-DD`
   - `--include-stale`
3. Result output:
   - `validity.state`, `validity.age_days`, `validity.stale_reason`
   - `score_breakdown.freshness_boost`
   - `score_breakdown.confidence_boost`
   - `score_breakdown.status_boost`
   - `score_breakdown.evidence_count_boost`
4. Default policy:
   - hide stale/future/expired/superseded evidence unless `--include-stale`
   - let `--as-of` surface evidence valid at the requested historical date
   - keep temporal boosts bounded so semantic relevance is not overwhelmed

### P1 — Tiny temporal/quality filter DSL

Complete this slice by extending the existing whitelist filter DSL with temporal fields. Keep it deliberately small:

Allowed fields:

- Existing structural/quality fields: `path`, `title`, `heading`, `tag`, `has_code`, `has_link`, `has_task_list`, `has_incomplete_tasks`, `confidence`, `status`, `source_type`
- New temporal fields: `valid_from`, `valid_until`, `updated`, `supersedes`, `superseded_by`

Allowed operators remain only:

- `==`
- `!=`
- `contains`
- flat `&&`

Explicit non-goals:

- no OR, parentheses, arithmetic, `now()`, list comprehensions, date math, or unbounded SQL fragments
- no computed `validity.state` filter in SQL DSL; validity remains the deterministic post-query temporal gate controlled by `--as-of` / `--include-stale`

Acceptance:

- RED/GREEN tests prove temporal fields parse and compile as parameterized SQL.
- Query tests prove `valid_from`, `updated`, and `superseded_by` filters work on both candidate paths without post-limit filtering leaks.
- Injection-like filter values remain parameters, not SQL fragments.

### P2 — Read-only agent/MCP surface for temporal recall

Complete this slice by making the read-only agent surface explicit and useful:

1. MCP `search` schema must document temporal filter examples and the temporal flags.
2. MCP tool list must remain read-only: only `search`, `get`, `status`, `health`.
3. Compact `search --view index` cards must expose enough temporal/evidence metadata for an agent to decide which chunk to fetch, without leaking full text:
   - `validity`
   - `quality` summary
   - `evidence_summary`
   - no `snippet`, no `text`, no `context_chunks`

Acceptance:

- MCP tool-surface tests assert no write tools appear.
- MCP schema tests assert temporal controls and temporal filter examples exist.
- Compact index tests assert temporal/evidence cards are present while text/snippet/context stay absent.

### P3 — Snippet/evidence polish

Complete this slice by improving result readability without changing retrieval semantics:

1. Add a small structured `evidence_summary` to full results and compact index cards:
   - validity state/reason/age
   - confidence/status/source_type
   - source count/kinds
   - evidence/open URI already present
2. Add a small structured `quality` summary to full results and compact index cards:
   - freshness/confidence/status/evidence-count boosts
   - total quality boost

Acceptance:

- Full result tests prove `evidence_summary` and `quality` match existing fields and do not require LLM generation.
- Compact index tests prove the same summary exists without thick text.
- Existing snippet behavior remains unchanged except for being accompanied by structured evidence.

## Hard boundaries

- No note generation or editing.
- No retrieval-count updates or strength mutation during search.
- No LLM rewrite/rerank/extraction dependency.
- No graph database or daemon.
- Public `search` and `get` keep `SQLITE_OPEN_READ_ONLY` and must not migrate databases.
- Schema migration is allowed only in writable `init`/`index` paths.
- No new persistence beyond existing SQLite sidecar schema and existing JSON outputs.

## Verification gates

1. TDD:
   - RED tests fail before implementation for P1/P2/P3.
   - GREEN tests pass after minimal implementation.
2. Rust gates:
   - `cargo fmt --check`
   - `cargo clippy --workspace -- -D warnings`
   - `cargo test -p orderk-core -- --nocapture`
   - `cargo test -p orderk-cli -- --nocapture`
   - `cargo test --workspace -- --nocapture`
3. Product gates:
   - `python3 scripts/release_gate.py`
   - stable artifact build + install after verification
   - live smoke using `/home/agent/.local/bin/orderk`, not `target/`
   - branch committed and pushed
   - temporary kitchen cleaned
4. Review/learning gates:
   - independent diff review PASS
   - rust-lean-tool-growth-loop reference updated with P0-P3 outcome and pitfalls
