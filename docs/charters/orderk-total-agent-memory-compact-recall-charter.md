# orderk x total-agent-memory Compact Recall Charter

Date: 2026-05-18
Source: `vbcherepanov/total-agent-memory` at commit `6e38b18`
Target baseline: orderk `986b66f`, v0.1.6
Rollback point: tag `pre-steal-total-agent-memory-20260518-083624`, branch `steal/total-agent-memory-compact-recall-20260518-083624`
Rollback command: `git switch main && git branch -D steal/total-agent-memory-compact-recall-20260518-083624` or `git reset --hard 986b66f` on the feature branch.

## Target boundary

orderk remains a tiny, local, read-only retrieval blade for Obsidian Markdown vaults. It must not write notes, generate summaries, run a background memory daemon, or become a second brain. The index is disposable SQLite sidecar state; Markdown remains source of truth.

Explicit non-goals preserved:

- no chat;
- no agent orchestration;
- no note writing;
- no automatic summaries;
- no LLM reranking;
- no second-brain lifecycle management;
- no save/forget/update memory tools;
- no daemon or reflection loop.

## Source atomic dissection

| Source subsystem | Evidence | User pain | Design choice | Value lever | Cost / do-not-copy | Minimal orderk slice |
|---|---|---|---|---|---|---|
| Progressive disclosure recall | `src/recall_modes.py:1-14`, `CHANGELOG.md:269-272` | Agents waste tokens when every search returns full content. | `memory_recall(mode="index")` returns compact id/title/score metadata; `memory_get(ids=[...])` fetches chosen full records. | 80-90% / ~83% token saving; agent chooses before paying for context. | Do not copy memory OS, save/update/delete, timeline/session machinery, LLM reflection, 60+ MCP tools. | Add `orderk search --view index` compact response + `orderk get --ids/--chunk-id` explicit full chunk fetch. |
| Index mode contract | `src/recall_modes.py:46-104`, `tests/test_recall_modes.py:39-55` | Need a lightweight first pass safe for agent prompts. | Strip per-item payload to id/title/score/type/project/created_at; sort flat hits by score; no content/context leakage. | Predictable, tiny, easy to parse. | Do not copy timeline neighbors yet; orderk already has explicit `--context-chunks` for thick evidence. | Compact entries include `chunk_id`, `title`, `score`, `path`, `heading`, `line_start`, `line_end`, and no `snippet`/`text`/`context_chunks`. |
| Fetch-by-id contract | `src/server.py:3925-3934`, `src/server.py:5452-5514`, `tests/test_memory_get.py` | After seeing an index, caller needs exact full evidence for selected IDs only. | Normalize/dedupe IDs, cap at 50, preserve caller order, skip missing IDs, detail full/summary. | Bounded output and deterministic user path. | Do not copy write/delete/update/history/relation tools. | `orderk get --ids a,b` / `--chunk-id x`, cap 50, preserve order, return full indexed chunk text and metadata from read-only DB. |

## Absorb / Adapt / Reject

### Absorb

1. **Two-stage retrieval contract**
   - Source value: index first, get exact payload later.
   - orderk path: `search --view index` + `get` command and MCP `get` tool.
   - Verification: compact result omits `snippet`, `text`, and `context_chunks`; `get` returns content for exactly requested chunk IDs.

2. **Bounded batched get**
   - Source value: max 50 IDs bounds context size and SQL expansion.
   - orderk path: dedupe IDs, keep caller order, cap at 50, skip missing.
   - Verification: unit/CLI contract tests.

### Adapt

1. **Index fields**
   - Source fields are memory-specific (`type`, `project`, `created_at`).
   - orderk fields are vault-specific: `chunk_id`, `title`, `score`, `path`, `heading`, `line_start`, `line_end`.

2. **MCP surface**
   - Source has many memory tools. orderk adapts only one read-only `get` tool beside existing `search/status/health`.

### Reject

- `memory_save`, `memory_update`, `memory_delete`, `memory_history`, `memory_relate` — write/lifecycle surface.
- Reflection daemon, auto-compression, activeContext projection, task phase state machine — hidden autonomy / second-brain lifecycle.
- CrossEncoder/LLM reranking and graph control plane — heavy and outside orderk's deterministic read-only boundary.
- Timeline/session browse mode — useful in total-agent-memory, but orderk already models authored Markdown chunks rather than chat sessions.

## Implementation plan

1. Add core response structs and read-only helpers:
   - `SearchIndexResponse` / `SearchIndexEntry` from an existing `QueryResponse`.
   - `ChunkGetOptions`, `ChunkGetResponse`, `ChunkGetResult`.
   - `get_chunks(db_path, ids, options)` opens SQLite read-only.
   - Public search/get paths open the existing SQLite index read-only; stale schema must be rebuilt or migrated explicitly via index/init, not silently during recall.
2. Add CLI:
   - `search --view full|index` default `full`.
   - `get --db ... --chunk-id ...` and `get --db ... --ids comma,separated`.
   - `get --detail full|summary`, default `full`.
3. Add MCP:
   - `search` accepts `view: "full" | "index"`.
   - `get` read-only tool with `ids`, `detail`, `context_chunks`.
4. Docs:
   - README quick-start mentions two-stage flow.
   - MCP read-only list becomes `search`, `get`, `status`, `health`.
5. Release/verification:
   - Rust unit tests first.
   - Cargo fmt/clippy/test/build.
   - Python contract/smoke/eval/release gate if feasible.
   - Stable artifact install + live DB smoke.

## Go / no-go gates

- Compact index output must not include body text (`snippet`, `text`) or neighbor context.
- `get` must use read-only DB open and must not call index/feedback/migration/write paths.
- Public `search` / MCP `search` must use read-only DB open and must not silently migrate the SQLite sidecar.
- Missing IDs are skipped; CLI/MCP `get` require at least one explicit ID, while the core helper normalizes an empty batch to an empty result.
- Batch max 50.
- MCP remains explicitly read-only: no index, maintain, feedback, save, forget, note-write, or chat tools.
- No secrets in output or docs.
