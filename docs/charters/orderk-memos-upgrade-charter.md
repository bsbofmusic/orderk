# orderk × Memos-Inspired Upgrade Charter

> **For Hermes:** This is a planning/architecture charter, not an implementation patch. Use `subagent-driven-development` only after this charter passes audit.

**Goal:** Absorb selected Memos engineering strengths into orderk while preserving orderk as a Rust, headless, read-only retrieval blade for Obsidian Markdown vaults.

**Architecture:** Obsidian remains the source of truth and writing surface. orderk scans Markdown read-only, writes only its own disposable SQLite index under `.obsidian/orderk/`, and returns ranked evidence through CLI / future read-only MCP. Memos is used only as a benchmark for structured metadata, filter semantics, and agent-access boundaries; it is not a product template to clone.

**Tech Stack:** Rust 2021, `rusqlite`, SQLite FTS5, `sqlite-vec`, existing orderk CLI, optional future Rust MCP crate.

**Audit status:** Revision 3 incorporates read-only audit findings from positioning/Obsidian compatibility, Rust feasibility, and verification/recovery reviewers. The charter is ready as a planning artifact; implementation still requires a fresh task-by-task implementation plan and normal review gates.

---

## 0. Non-Negotiable Positioning

orderk must remain:

- **Rust-first**: core features implemented in Rust; no Go/React runtime dependency.
- **Headless-first**: CLI/API/agent automation, not a full Obsidian UI replacement.
- **Read-only toward Markdown**: scan/read/chunk/embed Markdown; never write, edit, rename, delete, summarize into, or auto-tag source notes.
- **Disposable-index based**: `.obsidian/orderk/orderk.sqlite` is cache/index, not knowledge source.
- **Retrieval blade only**: search, filter, rank, evidence, health, eval; no chat app, memo app, workflow OS, or second-brain lifecycle manager.

### Explicit Non-Goals

Do **not** import or recreate these Memos surfaces:

- Web UI / React product shell
- memo CRUD as a product feature
- multi-user auth / OAuth / RBAC
- comments, reactions, activity feeds, webhooks
- attachment/resource manager
- automatic note generation or summaries written back to vault
- background daemon that silently mutates Markdown

---

## 1. Current orderk Baseline

Verified current shape:

- Core Rust repo: `crates/orderk-core`, `crates/orderk-cli`
- Current DB pipeline: `Obsidian vault -> scan -> parse -> chunk -> embed -> SQLite (FTS5 + sqlite-vec) -> JSON results`
- Current verification scripts exist under `scripts/`:
  - `scripts/contract.py`
  - `scripts/smoke.py`
  - `scripts/stress.py`
  - `scripts/eval.py`
  - `scripts/release_gate.py`
- Current status on live vault:
  - `notes`: 1025
  - `chunks`: 5833
  - `embeddings`: 5833
  - `embedding_provider`: `siliconflow`
  - `embedding_model`: `BAAI/bge-m3`
  - `embedding_dim`: 1024
  - `vector_backend`: `sqlite_vec`
  - state: ready

Key existing files:

- `crates/orderk-core/src/models.rs` — public structs and response models
- `crates/orderk-core/src/markdown.rs` — frontmatter tags, inline tags, wikilinks
- `crates/orderk-core/src/chunker.rs` — heading-aware chunking and stable chunk IDs
- `crates/orderk-core/src/index.rs` — SQLite schema, indexing, hybrid retrieval, scoring
- `crates/orderk-core/src/api.rs` — public API wrappers
- `crates/orderk-cli/src/main.rs` — CLI commands and argument parsing

---

## 2. Memos Lessons Worth Absorbing

Memos features audited as transferable patterns:

1. **Structured metadata payload**
   - Memos precomputes booleans such as task/list/code/link signals.
   - orderk can store similar facts per chunk in SQLite.

2. **Filterable search**
   - Memos uses a CEL-based filter engine to translate safe field predicates into SQL.
   - orderk should start with a much smaller Rust DSL to avoid complexity creep.

3. **Agent-callable boundary, parked**
   - Memos has MCP tools/resources.
   - For orderk, this is **not part of the current CLI-first upgrade path**.
   - Any MCP work is parked as a future optional adapter only after CLI semantics are stable.

4. **Better evidence snippets**
   - Memos-style search UX suggests richer snippet extraction.
   - orderk can improve snippets without changing core positioning.

---

## 3. Upgrade Roadmap

### Phase P0 — Chunk Structural Metadata

**Objective:** Add low-risk, read-only-derived metadata to chunk records.

**New chunk fields:**

```rust
has_code: bool,
has_link: bool,
has_task_list: bool,
has_incomplete_tasks: bool,
```

**Files:**

- Modify: `crates/orderk-core/src/models.rs`
- Modify: `crates/orderk-core/src/chunker.rs`
- Modify: `crates/orderk-core/src/index.rs`
- Tests: existing unit tests in `chunker.rs` and `index.rs`

**Behavior:**

- Detect fenced code blocks: triple backticks or tildes after leading whitespace.
- Detect links: `http://`, `https://`, Markdown links, and wikilinks.
- Detect task lists: `- [ ]`, `- [x]`, `- [X]`, `* [ ]`, `* [x]`, `* [X]` after leading whitespace.
- Detect incomplete tasks: unchecked variants only.
- Persist these booleans in `chunks` table.
- Do **not** expose these fields in `SearchResult` in P0 unless a later downstream contract explicitly requires it; keep the JSON contract small.

#### P0 Required DB Migration Strategy

This phase must support existing `.sqlite` files. `CREATE TABLE IF NOT EXISTS` is insufficient because it does not add columns to existing tables.

Required strategy:

- Add a schema/version marker using either `PRAGMA user_version` or an explicit `settings` key such as `schema_version`.
- Inspect existing columns with `PRAGMA table_info(chunks)` before migration.
- Add each new boolean column idempotently, for example:

```sql
ALTER TABLE chunks ADD COLUMN has_code INTEGER NOT NULL DEFAULT 0;
ALTER TABLE chunks ADD COLUMN has_link INTEGER NOT NULL DEFAULT 0;
ALTER TABLE chunks ADD COLUMN has_task_list INTEGER NOT NULL DEFAULT 0;
ALTER TABLE chunks ADD COLUMN has_incomplete_tasks INTEGER NOT NULL DEFAULT 0;
```

- Backfill existing rows from `chunks.text` using the Rust detector without reading or modifying Markdown files.
- If backfill cannot be completed safely, return a structured error that tells the user to run `orderk index`; do not silently treat old rows as false.
- Migration must be idempotent and covered by an old-schema fixture test.

**Acceptance Criteria:**

- Existing tests pass.
- New chunker test proves metadata detection.
- Old-schema SQLite fixture upgrades without data loss.
- Backfill from existing `chunks.text` works or fails loudly with remediation.
- Reindex preserves existing provider/model/dim/backend profile safety.
- No Markdown file is modified.
- DB migration is safe for existing SQLite files.

---

### Phase P1 — Minimal Filter DSL

**Objective:** Allow agent/user queries to constrain search using safe structured fields.

**Initial filter examples:**

```text
tag == "rust"
has_code == true
has_incomplete_tasks == true
path contains "brain/"
heading contains "项目"
tag == "rust" && has_code == true
```

**Supported in v1:**

- Operators: `==`, `!=`, `contains`, `&&`
- Only flat conjunctions with `&&`; no nested expressions.
- Field whitelist:
  - `path`
  - `title`
  - `heading`
  - `tag`
  - `has_code`
  - `has_link`
  - `has_task_list`
  - `has_incomplete_tasks`
- Value types: string, bool
- Quoting: support both single and double quoted strings with explicit escaping tests.
- Booleans: only lowercase `true` and `false` in P1.
- Maximum filter length: implementation must set a small hard cap before parsing.

**Operator-field matrix:**

| Field | `==` | `!=` | `contains` |
|---|---:|---:|---:|
| `path` | string | string | string |
| `title` | string | string | string |
| `heading` | string | string | string |
| `tag` | string exact membership | string exact non-membership | not supported in P1 |
| `has_code` | bool | bool | not supported |
| `has_link` | bool | bool | not supported |
| `has_task_list` | bool | bool | not supported |
| `has_incomplete_tasks` | bool | bool | not supported |

**Not supported in v1:**

- `||`
- parentheses
- `now()`
- arithmetic
- arbitrary SQL
- user-defined functions
- list comprehensions
- `tag contains ...`
- uppercase bools or unquoted string values

**Files:**

- Create: `crates/orderk-core/src/filter.rs`
- Modify: `crates/orderk-core/src/lib.rs`
- Modify: `crates/orderk-core/src/index.rs`
- Modify: `crates/orderk-core/src/api.rs`
- Modify: `crates/orderk-cli/src/main.rs`

**CLI target:**

```bash
orderk search \
  --db /path/to/orderk.sqlite \
  --query "sqlite vec" \
  --filter "tag == 'rust' && has_code == true"
```

#### P1 Filter Semantics

- `tag` operates on orderk-derived, read-only parsed tags. It does **not** claim full parity with Obsidian native tag search semantics.
- Tag filtering must be exact membership, not substring matching against JSON text.
- Preferred implementation for current `tags_json TEXT`: use SQLite JSON support such as `json_each(tags_json)` if available and tested in bundled SQLite.
- If `json_each` is unavailable or unreliable, create/backfill a lightweight normalized `chunk_tags(chunk_rowid, tag)` table instead of using raw `LIKE '%rust%'` against JSON.
- Any tag normalization affects only the SQLite index; never write normalized tags back to Markdown/frontmatter.
- `contains` should use parameterized values such as `instr(lower(coalesce(column,'')), lower(?)) > 0`; field/column names must be hardcoded from the whitelist.

#### P1 Filter Execution Plan

The implementation must avoid naive final post-filtering that can silently lose relevant results.

Required execution rules:

1. Parse filter once into an AST.
2. Compile to a safe SQL predicate with parameterized values.
3. Push down SQL-filterable predicates into candidate retrieval whenever possible:
   - FTS keyword candidate SQL
   - route/path/title/heading candidate SQL
   - exact backend row scan SQL
4. For `sqlite-vec`, verify whether rowid prefiltering is possible with the installed `sqlite-vec` version. If not, use a bounded overfetch + postfilter fallback and document that fallback in routing evidence.
5. Never truncate to final `limit` before applying the filter.
6. Routing evidence should record filtered candidate counts when filters are used.
7. Empty filter must be identical to current no-filter search behavior.

**Safety Rules:**

- Filter parser must use a strict field whitelist.
- SQL must be parameterized; no string interpolation of values.
- Unknown fields/operators fail closed with `E_INVALID_ARGUMENT`-style structured error JSON.
- Type mismatch fails closed.
- SQL injection-looking values must be treated as string values, never executable SQL.
- API should avoid breaking no-filter callers: either keep existing `query(...)` as a wrapper or introduce `SearchOptions` / `query_with_filter(...)`.

**Acceptance Criteria:**

- Existing `orderk search` behavior unchanged when `--filter` omitted.
- Invalid filters return structured error JSON mapped to invalid argument semantics.
- Tests cover allowed fields, unknown fields, unsupported operators, bool/string mismatch, bad quoting, SQL injection-looking input, exact tag membership, no substring tag false positives, and `&&` composition.
- Tests cover filter + hybrid search, filter + exact backend, no-filter compatibility, and high-selectivity filters.
- Filter does not reduce exact/original evidence quality when omitted.

---

### Phase P2 — Parked: Optional Read-Only MCP Adapter

**Status:** Parked. This phase is **not** part of the current CLI-first upgrade path.

**Why parked:** The user requirement is CLI-first. The immediate product should remain a native Rust CLI with stable JSON output. MCP is only a possible future adapter for environments that prefer protocol-native tool calls over spawning a CLI process.

**Current decision:** Do not implement MCP during the P0/P1/P3 upgrade. Keep all near-term work in CLI/core.

**If revisited later:**

- P2 must not start until CLI filter semantics and DB migration behavior are stable.
- MCP must be a thin adapter over already-stable CLI/core read APIs.
- MCP must not introduce a daemon requirement for normal orderk use.

**Candidate command, only if revived:**

```bash
orderk serve --mcp
```

**Allowed tools, only if revived:**

- `orderk.search(query, filter?, limit?)`
- `orderk.status()`
- `orderk.health(smoke_query?)`
- `orderk.list_tags(prefix?, limit?)`

**Forbidden tools:**

- `orderk.write_note`
- `orderk.update_note`
- `orderk.delete_note`
- `orderk.index` as a default MCP tool
- `orderk.auto_tag`
- `orderk.summarize_to_vault`

**Files / crate shape, only if revived:**

- Prefer create: `crates/orderk-mcp/`
- Modify: root `Cargo.toml`
- Modify: `crates/orderk-cli/src/main.rs` only for command wiring if needed

#### P2 Read-Only DB Rule

MCP read-only means more than “does not write Markdown.” MCP tools must not silently initialize, migrate, repair, reindex, or create the SQLite DB.

Required behavior if revived:

- MCP search/status/list_tags/health must use SQLite read-only open mode where feasible.
- If the DB is missing, profile mismatched, schema too old, or migration required, return a structured error with remediation instead of mutating the DB.
- `health(smoke_query?)` may run read-only smoke search only; it must not call index, init, repair, or migration.
- `list_tags` reads only the index; it must not scan the vault or generate tags.
- Tool `limit` values must have a hard maximum.
- Failure responses preserve structured error envelopes.

**Acceptance Criteria if revived:**

- MCP is read-only by design and by DB open mode where possible.
- MCP tools call existing `orderk_core` read APIs or newly added read-only wrappers.
- No Markdown writes.
- No SQLite init/migrate/reindex hidden behind MCP.
- No long-running index mutations hidden behind MCP.
- Failure responses preserve structured error envelope.

---

### Phase P3 — Snippet Quality Upgrade

**Objective:** Improve result evidence without changing ranking semantics.

**Desired improvements:**

- Preserve original casing.
- Find multiple query term hits.
- Join multiple short windows when useful.
- Optionally emit highlight ranges or marker-free snippets.

**Files:**

- Modify: `crates/orderk-core/src/index.rs`
- Tests: add/extend snippet unit tests

**Acceptance Criteria:**

- UTF-8 safe.
- Existing ranking unchanged.
- Snippet has deterministic output.
- Tests cover Chinese text, emoji, multiline Markdown, and no-hit fallback.
- Snippet/highlight data is returned only as evidence; it never writes back to Markdown.

---

## 4. Obsidian Compatibility Rules

### Safe Integration Model

```text
Obsidian = source of truth + writing/editor UI
orderk = read-only scanner + disposable index + agent retrieval sidecar
```

### Conflict Avoidance

- Do not write Markdown.
- Do not modify frontmatter.
- Do not alter Obsidian native search behavior.
- Do not claim full parity with Obsidian native search query language.
- Do not depend on Obsidian being open.
- Do not assume Smart Connections is present.
- Do not read/write/reuse Smart Connections plugin DBs, embedding cache, or plugin config.
- Keep `.obsidian/orderk/` treated as cache/runtime output.
- Treat `.obsidian/orderk/**`, `*.sqlite`, `*.sqlite-wal`, and `*.sqlite-shm` as untracked runtime cache for Git/DR backup purposes.
- If Obsidian Sync or another file sync service is used, exclude `.obsidian/orderk/` where possible because the index is disposable and rebuildable.
- Ensure indexing jobs are serialized; SQLite supports many readers but should not have multiple concurrent rebuild writers.
- Do not introduce a background daemon that silently rebuilds the index or mutates Markdown.

---

## 5. Test, Release, and Verification Gates

Before any implementation PR/commit is considered complete, run the core gate:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --all-features --release
python3 scripts/contract.py
python3 scripts/smoke.py
python3 scripts/stress.py
python3 scripts/eval.py
python3 scripts/release_gate.py
```

If local `rustfmt` or `clippy` is unavailable, record the local limitation; CI/release still treats them as required gates.

If a phase touches the npm wrapper, Obsidian package, CLI argument contract consumed by JS, or release packaging surface, also run the package gate from the README:

```bash
npm install
npm run build --workspaces --if-present
npm test --workspaces --if-present
npm pack --workspaces --dry-run
```

All phase-specific Rust tests below must be included in `cargo test --workspace --all-features`, not kept as manual-only checks.

Filter-specific gates:

- parser unit tests
- SQL generation tests
- injection safety tests
- search integration tests with and without filters
- invalid filter structured error test
- exact tag membership test
- tag substring false-positive test
- high-selectivity filter test proving results are not truncated before filtering
- old no-filter CLI/API compatibility tests

Migration-specific gates:

- old-schema fixture test
- idempotent migration test
- metadata backfill test from existing `chunks.text`
- no-profile-mismatch regression test
- migration failure returns structured remediation without touching Markdown

Obsidian safety gates:

- `git status --short /home/agent/obsidian-vault` before/after test run if live vault is used
- If no live vault is used, run the same safety assertions against a fixture vault
- Verify no `.md` files changed
- Verify `.obsidian/orderk/` remains cache-only and untracked
- Verify no orderk SQLite/WAL/SHM files are staged or tracked in vault Git

MCP-specific gates, only if the parked P2 adapter is revived later:

- read-only DB open test
- missing DB returns structured error without creating files
- old schema returns remediation without migration
- tool limit cap test
- allowlist test: only search/status/health/list_tags exist
- protocol smoke test: start `orderk serve --mcp`, list tools, call allowed read-only tools, confirm forbidden tools are absent

Secret/log safety gates:

- Error JSON, routing evidence, MCP errors, logs, and release artifacts must not include API keys or environment secret values.
- Add or extend tests for provider/key error paths if any code touches logging or error formatting.

### Rollback / Recovery Gates

SQLite index recovery must be explicit because P0/P1 change DB semantics.

Required recovery properties:

- Migration failure must not modify Markdown files.
- Migration failure must return a structured error with remediation.
- The operator must be able to recover by deleting/rebuilding `.obsidian/orderk/orderk.sqlite` because it is disposable cache.
- If migration writes are not atomic, implementation must create a safe backup or use a transaction so failure cannot leave a half-migrated DB without remediation.
- Release notes for schema-changing phases must document whether reindex is required.
- Tests must cover: migration success, migration idempotency, migration failure/remediation, and rebuild-from-empty DB.

---

## 6. Suggested Implementation TODOs

1. **P0 schema + model design**
   - Define schema version strategy (`PRAGMA user_version` or `settings.schema_version`).
   - Define idempotent column migration.
   - Decide that P0 booleans do not appear in `SearchResult` by default.

2. **P0 metadata detector tests**
   - Add chunker tests for code/link/task/incomplete task detection.
   - Include `[X]`, leading whitespace, Chinese surrounding text, and no false positive cases.

3. **P0 metadata implementation**
   - Add fields to `Chunk`.
   - Populate fields in `push_chunk()`.
   - Persist fields in SQLite.
   - Backfill old rows from `chunks.text`.
   - Extend index roundtrip tests.

4. **P1 filter parser design**
   - Define `FilterExpr`, `FilterValue`, parse errors, field schema, operator-field matrix, filter length cap, and quoting rules.

5. **P1 filter SQL compiler**
   - Compile only whitelisted fields to parameterized SQL fragments.
   - Implement exact tag membership via `json_each(tags_json)` or normalized `chunk_tags` table.
   - Add injection-looking tests.

6. **P1 query integration**
   - Thread `filter: Option<&str>` through CLI → API → IndexStore without breaking no-filter callers.
   - Push down SQL-filterable predicates where possible.
   - Avoid final-limit truncation before filtering.
   - Add filtered candidate counts to routing evidence.

7. **P1 filter docs**
   - Document supported v1 syntax, unsupported syntax, tag semantics, and no parity claim with Obsidian native search.

8. **P2 MCP parking-lot note**
   - Do not implement MCP in the current CLI-first upgrade.
   - Revisit only after P0/P1/P3 are stable and there is a concrete protocol-adapter need.
   - If revived later, define read-only DB wrappers before exposing tools.

9. **P3 snippet upgrade**
    - Improve snippet extraction with UTF-8-safe multi-hit logic.
    - Preserve original casing.

11. **Final integration gate**
    - Run full Rust + script verification suite.
    - Run npm/package gates if any affected surface touches wrapper/plugin/package behavior.
    - Confirm rollback/rebuild path for schema-changing phases.
    - Confirm logs/errors do not leak API keys.
    - Confirm Obsidian vault remains unmodified except disposable index updates if explicitly reindexed.

---

## 7. Audit Questions Before Implementation

Subagents should answer these before code work begins:

1. Does this charter preserve orderk's README positioning?
2. Does any phase risk turning orderk into a note-writing or memo-management app?
3. Are the Obsidian compatibility rules sufficient?
4. Is the minimal filter DSL small enough, or should it be narrower?
5. Are P0 migrations safe for existing `.sqlite` files?
6. Is tag exact-membership filtering implemented without JSON substring false positives?
7. Does P1 avoid post-limit filtering that silently drops valid results?
8. Which tests are mandatory before merging P0/P1?
9. Should MCP remain parked unless a future non-CLI adapter requirement appears?
10. Do all verification commands exist and run in this repo?

---

## 8. Implementation Go / No-Go Criteria

Proceed to implementation only if audits agree that:

- P0 and P1 do not write Markdown.
- P0 has an idempotent, tested old-DB migration path.
- P1 filter DSL is field-whitelisted, operator-restricted, length-capped, and parameterized.
- `tag == ...` is exact membership, not JSON substring matching.
- Filter execution avoids truncating before filtering.
- Obsidian remains the source of truth.
- `.obsidian/orderk/` is treated as disposable cache and excluded from source-control/DR backup.
- MCP remains parked unless a future non-CLI adapter requirement appears; if revived later, it must be read-only by DB open mode and tool allowlist.
- No Go/React/Memos runtime dependencies are introduced.
- Verification, release, and rollback gates are explicit and runnable.

If any audit finds a positioning, migration, filter-correctness, or gate violation, fix this charter before writing code.
