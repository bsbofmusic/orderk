# orderk

orderk is a tiny, local, read-only retrieval blade for Obsidian Markdown vaults.

It turns your vault into fast, structured evidence for humans, scripts, and AI agents. It does not write notes, generate summaries, run a background memory daemon, or try to become your second brain.

Search your vault, feed agents grounded context, and keep your notes untouched.

## What it is

A fast, stable, low-overhead search blade:

```text
Obsidian vault -> scan -> parse -> chunk -> embed -> SQLite (FTS5 + sqlite-vec) -> JSON results
```

Obsidian remains the place where knowledge lives. orderk is the retrieval layer beside it: small enough to forget, structured enough for automation, and strict enough to trust with agent workflows.

The name is half serious: `order` for structure, `k` for knowledge at speed.

## Why orderk

Most knowledge tools try to become the place where your thinking happens. orderk does not. Your Markdown files remain the source of truth. orderk builds a disposable local SQLite index beside them and returns evidence through a CLI, JSON output, and a thin read-only MCP surface.

It is built for people who want better recall without giving another app permission to rewrite their vault, run a hosted memory system, or turn search into a chat product.

### Feature -> benefit

| Feature | Benefit |
|---|---|
| Single Rust binary | Small surface area and fast startup. The installed Linux x64 binary for v0.1.9 is 23,886,168 bytes, about 22.8 MiB, under a 30 MiB release-gate ceiling. |
| On-demand CLI | No always-on orderk daemon. Normal runtime is one command, one result, then exit. |
| Low runtime memory | A live search probe on the maintainer machine used about 9.2 MiB VmRSS and 12.3 MiB VmPeak, under a 15 MiB baseline ceiling. |
| Disposable SQLite index | Files, chunks, embeddings, FTS, vector rows, settings, and feedback live in one rebuildable DB. Delete the index and your Markdown vault is still intact. |
| Hybrid retrieval | Keyword search, vector search, query-aware routing, path/tag/recency signals, and RRF-style fusion work together instead of pretending one score explains everything. |
| Metadata-aware reranking | Paths, headings, tags, frontmatter, confidence, status, source type, and link evidence can influence rank without an LLM rewrite step. |
| Retrieval workflow controls | Opt-in chunk overlap, deterministic query expansion, JSON Lines output, eval A/B, and a bounded lexical reranker give agents and maintainers more control without widening the read-only boundary. |
| Obsidian-native evidence | Results can include snippets, headings, tags, wikilinks, backlinks, neighbor chunks, score breakdowns, and routing timings, so agents can explain what they found. |
| Cheap embeddings by default | Production defaults use SiliconFlow + `BAAI/bge-m3` (`1024` dimensions), which is strong enough for everyday personal-vault recall without paying for a large memory platform. |
| Read-only agent surface | CLI search and MCP expose retrieval, status, and health. They do not expose note writing, save, forget, chat, or index mutation tools. |
| Obsidian workflow stays intact | No migration, no hosted workspace, no lock-in. Try it, rebuild it, or remove it without changing your notes. |

### Benchmark snapshot

These are maintainer-machine examples, not universal promises. They exist to make the "lightweight" claim falsifiable. Full reports live in [`benchmarks/`](benchmarks/).

| Metric | Example result |
|---|---:|
| Installed binary | 23,716,616 bytes (~22.6 MiB) |
| Release binary budget | <= 30 MiB |
| Status / live-search RSS | ~9.2 MiB measured VmRSS |
| Resident orderk daemon count | 0 |
| Live vault notes | 2,306 |
| Live vault chunks / embeddings | 19,081 / 19,081 |
| Live SQLite index size | ~215 MiB |
| Offline fixture eval | 4/4 top-1 hits; recall@k, nDCG, and MRR all 1.0 |
| Live eval gate | 5/5 hits; MRR 0.68 |
| Mock stress p50 / p95 | 72.4ms / 96.2ms |
| Live semantic search example | ~1.6s including the remote SiliconFlow embedding call |

The local index path is millisecond-level. Live semantic search also asks the embedding provider for a fresh query vector, so network/provider latency can dominate. The point is simple: use cheap embeddings for recall, save expensive LLM calls for reasoning.

### Token savings: compact recall before full evidence

orderk does not ask agents to swallow the whole vault. It uses progressive disclosure:

```text
search --view index -> inspect candidate cards -> get selected chunks
```

| Query | Full search bytes | `--view index` bytes | Selected `get` bytes | Top file preserved? |
|---|---:|---:|---:|---|
| `orderk retrieval blade` | 22,830 | 13,400 | 2,996 | yes |
| `Obsidian graph rules` | 22,364 | 12,757 | 2,285 | yes |

### Retrieval workflow controls

These knobs are opt-in and do not change the source of truth or the read-only boundary.

```bash
orderk index \
  --vault /path/to/vault \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --chunk-max-chars 1200 \
  --chunk-overlap 80

orderk search \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --query "retrieval workflow notes" \
  --query-expansion \
  --reranker lexical \
  --json-lines

orderk eval \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --queries /path/to/eval-queries.json \
  --vault /path/to/vault \
  --ab-chunk-overlap 80
```

- `--chunk-overlap` preserves boundary context when chunks are size-capped.
- `--query-expansion` uses a deterministic lexical map, not an LLM rewrite.
- `--json-lines` emits one JSON object per line for piping and tooling.
- `--reranker lexical|none` enables or disables a bounded deterministic lexical second pass; `--no-rerank` only disables the metadata-aware rerank path.
- `--ab-chunk-overlap` compares overlap settings against the baseline eval run.

In representative live-vault queries, compact recall cut output size by **41–46%** while preserving the same top file. See [`benchmarks/TOKEN_SAVINGS.md`](benchmarks/TOKEN_SAVINGS.md).

### vs alternatives

orderk is not a memory OS. It is the small retrieval layer beside your existing Markdown vault.

| Dimension | orderk | agentmemory / mem0 / Letta class tools | Built-in note search |
|---|---|---|---|
| Type | Local retrieval blade | Memory engine / API / agent runtime | App search |
| Source of truth | Markdown vault | Memory DB / service-runtime layer | Markdown vault |
| Writes notes | No | memory writes / API state | manual only |
| Daemon/server | No | common | app runtime only |
| Search | BM25 + vector + metadata + links | vector / graph varies | keyword / app index |
| Agent surface | CLI + read-only MCP | MCP / REST / hooks / runtime | none or manual |
| Token control | `--view index` + `get --ids` | memory budget / integration-dependent | manual / context-heavy |
| Best for | Grounded evidence retrieval | Persistent memory lifecycle | Human note search |

If you want a memory operating system, use a memory system. If you want a small, local, read-only retrieval blade for your Markdown vault, use orderk. See [`benchmarks/COMPARISON.md`](benchmarks/COMPARISON.md).

### Fast, precise, sharp, stable

| Constraint | How orderk handles it |
|---|---|
| Fast | Local SQLite index, small Rust binary, JSON-first CLI, no resident service. |
| Precise | Hybrid keyword/vector retrieval, deterministic metadata boosts, Obsidian link evidence, and checked eval fixtures. |
| Sharp | One job only: retrieve grounded evidence from your vault. |
| Stable | Read-only design, profile guards, deterministic local ranking, disposable index, health/doctor/maintain gates. |

### Smarter by stealing less

orderk borrows useful ideas from memory tools without becoming one.

- From Obsidian: local Markdown, links, headings, tags, and frontmatter.
- From vector search: semantic recall.
- From agent memory systems: structured retrieval APIs and evidence-rich context.
- From ranking systems: fusion, metadata boosts, link expansion, and deterministic reranking.

It rejects the heavy parts: note generation, automatic memory mutation, chat, hosted source of truth, hidden reflection loops, and always-on memory daemons.

## Architecture

```text
Agent
  -> orderk CLI
    -> orderk-core
      -> vault scanner
      -> markdown parser
      -> chunker
      -> embedding provider (SiliconFlow by default)
      -> SQLite store
         -> files / chunks / chunk_embeddings / settings / feedback_events
         -> FTS5 keyword index
         -> sqlite-vec vector index
      -> hybrid retriever
      -> JSON response
```

### Core modules

| Module | Responsibility |
|---|---|
| `crates/orderk-core` | scan, parse, chunk, embed, store, rank, return JSON |
| `crates/orderk-cli` | native CLI entrypoint for index/search/status/health/doctor/eval/maintain/capsule/feedback |
| `packages/cli` | npm wrapper that finds or downloads the native binary |
| `packages/obsidian` | thin Obsidian desktop plugin wrapper |

## Which package do I need?

| Need | Use |
|---|---|
| Native CLI for local / agent use | `cargo install --path crates/orderk-cli --locked` |
| JavaScript entrypoint + Linux x64 one-click path | `npm install -g orderk-cli` |
| Obsidian desktop plugin | Source-only wrapper in `packages/obsidian`; not published as a maintained npm package |
| Core Rust retrieval engine | `crates/orderk-core` |

## Production defaults

- **Embedding provider**: `siliconflow`
- **Embedding model**: `BAAI/bge-m3`
- **Embedding dimension**: `1024`
- **Vector backend**: `sqlite_vec`

Set one of these environment variables before indexing or searching:

- `HERMES_SILICONFLOW_API_KEY`
- `SILICONFLOW_API_KEY`

## Prerequisites

- Rust + Cargo if you want to build from source
- Node.js if you want the npm wrapper or local Obsidian plugin build
- Obsidian desktop only for the plugin wrapper
- A SiliconFlow API key for production embeddings
- Linux x64 if you want the one-click npm binary download path

## Quick start

### 1) Install

```bash
cargo install --path crates/orderk-cli --locked
# or
npm install -g orderk-cli
```

### 2) Export your embedding key

```bash
export HERMES_SILICONFLOW_API_KEY="..."
# or
export SILICONFLOW_API_KEY="..."
```

### 3) Index a vault

```bash
orderk index \
  --vault /path/to/vault \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024 \
  --vector-backend sqlite_vec
```

### 4) Search

```bash
orderk search \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --query "vector search for knowledge notes" \
  --limit 10 \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024 \
  --vector-backend sqlite_vec
```

Agent-facing compact recall, borrowed from the total-agent-memory audit:

```bash
# First pass: compact cards only, no snippets/full text.
orderk search \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --query "vector search for knowledge notes" \
  --view index \
  --limit 10 \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024 \
  --vector-backend sqlite_vec

# Second pass: fetch exact chunks chosen by chunk_id.
orderk get \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --ids chk_abc,chk_def \
  --detail full
```

`--view index` returns `orderk.search_index.v1` with `chunk_id`, `title`, `score`, `path`, `heading`, and line range only. It intentionally omits `snippet`, `text`, and neighbor context so agents can choose before spending tokens. Both public `search` and `get` open the existing SQLite index read-only; stale sidecar schemas should be rebuilt or migrated explicitly instead of being mutated during recall. `orderk get` returns `orderk.get.v1`, preserves requested ID order, caps batches at 50, and skips missing IDs.

Agent-facing evidence controls borrowed from the Supermemory audit:

```bash
orderk search \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --query "vector search for knowledge notes" \
  --limit 10 \
  --min-score 0.2 \
  --context-chunks 1 \
  --include-links \
  --expand-links 1 \
  --filter "confidence == 'high' && status == 'active'" \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024 \
  --vector-backend sqlite_vec
```

- `--min-score` / `--threshold`: drop low fused-score tails after candidate ranking.
- `--context-chunks N`: include before/after same-file chunk evidence.
- `--include-links`: include Obsidian wikilink/backlink evidence from indexed vault text.
- `--retrieval-depth 1`: include one-hop authored Obsidian wikilink/backlink candidates for deeper recall. Default `0` returns direct keyword/vector/route candidates only.
- `--expand-links 1`: compatibility alias for `--retrieval-depth 1`; off by default and capped at one hop.
- `--filter "tag == 'rust' && has_code == true && confidence == 'high'"`: optional metadata filter DSL. Supported fields are `path`, `title`, `heading`, `tag`, `has_code`, `has_link`, `has_task_list`, `has_incomplete_tasks`, `confidence`, `status`, and `source_type`.
- `--no-rerank`: disable the deterministic metadata-aware rerank path. By default orderk adds a bounded `score_breakdown.metadata_boost` from indexed structure/frontmatter.
- `--reranker lexical|none`: optionally run a bounded deterministic lexical reranker after temporal-quality adjustment.
- `--query-expansion`: enable deterministic lexical query expansion for short or synonym-heavy searches.
- `--json-lines`: emit one JSON object per line for scripts and pipes.

### 5) MCP read-only server

```bash
orderk mcp \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024 \
  --vector-backend sqlite_vec
```

The MCP surface is intentionally thin and read-only: `search`, `get`, `status`, and `health`. `search` supports `view: "index"` for compact id/title/score/path cards, and `get` explicitly fetches selected chunk IDs. Search/get open the existing SQLite index in read-only mode and do not run index, feedback, migration, maintenance, or note-write paths. It supports standard `Content-Length` stdio frames and a JSONL compatibility mode for simple smoke tests. It does not expose index, feedback, maintain, save, forget, note-write, or chat tools.

### 6) Inspect status

```bash
orderk status --db /path/to/vault/.obsidian/orderk/orderk.sqlite
```

### 7) Run health / doctor

```bash
orderk health \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --vault /path/to/vault \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024 \
  --vector-backend sqlite_vec

orderk doctor \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --vault /path/to/vault \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024 \
  --vector-backend sqlite_vec
```

Optional `--smoke-query` turns `doctor` into a retrieval smoke probe; without it, `doctor` behaves like `health`.

### 8) Run maintain

```bash
orderk maintain \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --vault /path/to/vault \
  --queries /path/to/eval-queries.json \
  --smoke-query "known phrase in your vault" \
  --limit 10 \
  --report-dir /tmp/orderk-reports \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024 \
  --vector-backend sqlite_vec
```

`maintain` emits `orderk.maintain.v1` JSON: nested health evidence, optional eval evidence, typed error codes, and a persisted report path when `--report-dir` is provided.

### 9) Export / inspect a capsule manifest

```bash
orderk capsule export \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --vault /path/to/vault \
  --out /path/to/orderk.capsule.json

orderk capsule inspect \
  --file /path/to/orderk.capsule.json \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite
```

A capsule manifest is a portable, self-describing JSON receipt for the SQLite index: schema/profile, note/chunk/embedding counts, DB + SQLite WAL/SHM sidecar size/checksum, and source vault pointer. It does not copy note contents, write Markdown, import/restore, expose MCP write tools, or replace the SQLite index. `capsule export --out` rejects paths inside the vault and paths that would overwrite the DB or SQLite sidecars.

### 10) Run eval

```bash
python3 scripts/eval.py
```

The eval script is a deterministic offline quality gate. It indexes the checked-in fixture vault at `fixtures/eval/vault`, runs `fixtures/eval/queries.json`, and compares the report against `baselines/orderk-eval-baseline.json`. The gate fails on missing fixtures, zero-hit cases, top-1 regressions, or metric regressions in `recall_at_k`, `ndcg_at_k`, `mrr`, and mean latency. Advanced/dev-only overrides are available through `ORDERK_EVAL_VAULT`, `ORDERK_EVAL_QUERIES`, and `ORDERK_EVAL_BASELINE`; release runs should use the checked-in defaults.

The CLI prints JSON by default. Search can also emit JSON Lines via `--json-lines` when a pipeline wants one result per line.

## Agent setup

This is the shortest path for an agent or automation:

1. Point `--vault` at the Obsidian vault.
2. Keep the SQLite DB inside the vault, usually under `.obsidian/orderk/orderk.sqlite`.
3. Use `siliconflow` as the embedding provider.
4. Set `BAAI/bge-m3` + `1024` unless you have a strong reason to change them.
5. Use `sqlite_vec` as the vector backend.
6. Consume the JSON output directly. Search responses include `route`, `routing` with per-stage timings, `retrieval_depth`, and link expansion counts, per-result `score_breakdown`, `evidence` with `evidence_count` and per-result `retrieval_depth`, `tags`, `confidence`, `status`, `source_type`, optional neighbor `context_chunks`, and optional Obsidian link evidence.
7. Use `--view index` plus `orderk get --ids ...` for two-stage compact recall when an agent should inspect candidate IDs before fetching full text.
8. Use `--chunk-max-chars`, `--chunk-overlap`, `--query-expansion`, `--json-lines`, `--reranker lexical|none`, `--min-score`/`--threshold`, `--context-chunks`, `--include-links`, `--retrieval-depth 1`, `--expand-links 1`, `--filter`, and `--no-rerank` when an agent needs thicker evidence, deterministic lexical expansion, machine-friendly line output, or wants metadata rerank disabled.
9. If the client supports MCP, use `orderk mcp` for read-only `search`/`get`/`status`/`health` tools instead of asking the agent to guess shell flags.
10. Use `orderk capsule export` / `orderk capsule inspect` when an agent needs to verify that a portable SQLite index artifact still matches its recorded profile, counts, size, and checksum.
11. Use `orderk maintain --report-dir ...` as the agent-facing readiness/failure-ticket gate before release or scheduled checks.

### Obsidian plugin settings

The plugin is desktop-only and shells out to the native CLI.

Required settings:

- vault path
- CLI binary path, or `ORDERK_BIN`
- embedding provider
- embedding model
- embedding dimension
- search limit

## Verification

```bash
python3 -m unittest scripts/test_release_gate.py scripts/test_eval_gate.py scripts/test_feedback_to_eval.py
cargo fmt --all -- --check
cargo clippy --workspace --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --all-features --release
python3 scripts/contract.py
python3 scripts/smoke.py
python3 scripts/stress.py
python3 scripts/eval.py
python3 scripts/release_gate.py
npm install
npm run build --workspaces --if-present
npm test --workspaces --if-present
npm pack --workspace orderk-cli --dry-run
```

`python3 scripts/release_gate.py` is the canonical pre-publish gate. It also checks version consistency, secret/package cleanliness, release/eval/feedback-growth gate unit tests, the resource baseline in `baselines/orderk-resource-baseline.json`, and the eval quality baseline in `baselines/orderk-eval-baseline.json`.

## Troubleshooting

For the full maintenance contract, see [`docs/MAINTAIN.md`](docs/MAINTAIN.md). For failure triage, see [`docs/TROUBLESHOOTING.md`](docs/TROUBLESHOOTING.md).

| Symptom | Likely fix |
|---|---|
| `SiliconFlow API key is missing` | Export `HERMES_SILICONFLOW_API_KEY` or `SILICONFLOW_API_KEY` |
| `orderk CLI not found` | Install the native binary or set `ORDERK_BIN` |
| Search returns no vector hits | Re-index with matching provider / model / dim |
| `index profile mismatch` | Rebuild the SQLite DB with the same embedding provider, model, dimension, and backend |
| Obsidian plugin cannot find the binary | Set the binary path in plugin settings or use `ORDERK_BIN` |
| One-click npm install does nothing on macOS/Windows | The packaged binary path is Linux x64 first; use `cargo install` or a local binary |

## Security

- Do not commit API keys.
- Do not commit vault contents.
- Do not commit SQLite indexes.
- Keep secrets in environment variables or local app settings only.
- Never print key values in logs or commits.

## Release notes

- `orderk-cli` is the only maintained npm package and is the Node wrapper around the native binary.
- `orderk-obsidian` is legacy/deprecated on npm; the Obsidian wrapper source remains in `packages/obsidian`.
- Linux x64 one-click install is served from GitHub Releases.

## What orderk deliberately does not do

- chat
- agent orchestration
- note writing
- automatic summaries
- LLM / cross-encoder reranking; the only optional rerank path is the bounded deterministic lexical reranker
- second-brain style lifecycle management

## License

MIT
