# orderk

orderk is a headless, ultra-light vector search plugin for Obsidian Markdown vaults.

It is built for agents that need fast local retrieval.
It is not a chat app, not an agent orchestrator, and not a note-writing system.

## What it is

A fast, stable, low-overhead search blade:

```text
Obsidian vault -> scan -> parse -> chunk -> embed -> SQLite (FTS5 + sqlite-vec) -> JSON results
```

## Why orderk

- **Headless first**: designed for CLI / agent / plugin automation.
- **Rust core**: small surface area, fast startup, low memory.
- **Single-file storage**: one SQLite DB for files, chunks, embeddings, FTS, and feedback.
- **Hybrid retrieval**: keyword + vector + query-aware routing + path/tag/recency signals.
- **Cloud embeddings**: production path uses SiliconFlow + BAAI/bge-m3.
- **Obsidian-friendly**: keeps the vault workflow intact.
- **No product bloat**: no chat, no note generation, no second-brain OS.

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
| `crates/orderk-cli` | native CLI entrypoint for index/search/status/health/doctor/eval/maintain/feedback |
| `packages/cli` | npm wrapper that finds or downloads the native binary |
| `packages/obsidian` | thin Obsidian desktop plugin wrapper |

## Which package do I need?

| Need | Use |
|---|---|
| Native CLI for local / agent use | `cargo install --path crates/orderk-cli --locked` |
| JavaScript entrypoint + Linux x64 one-click path | `npm install -g orderk-cli` |
| Obsidian desktop plugin | `orderk-obsidian` |
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
- Node.js if you want the npm wrapper or Obsidian package
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

Agent-facing evidence controls borrowed from the Supermemory audit:

```bash
orderk search \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --query "vector search for knowledge notes" \
  --limit 10 \
  --min-score 0.2 \
  --context-chunks 1 \
  --include-links \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024 \
  --vector-backend sqlite_vec
```

- `--min-score` / `--threshold`: drop low fused-score tails after candidate ranking.
- `--context-chunks N`: include before/after same-file chunk evidence.
- `--include-links`: include Obsidian wikilink/backlink evidence from indexed vault text.
- `--filter "tag == 'rust' && has_code == true"`: optional metadata filter DSL.

### 5) MCP read-only server

```bash
orderk mcp \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024 \
  --vector-backend sqlite_vec
```

The MCP surface is intentionally thin and read-only: `search`, `status`, and `health`. It supports standard `Content-Length` stdio frames and a JSONL compatibility mode for simple smoke tests. It does not expose index, feedback, maintain, save, forget, note-write, or chat tools.

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

### 9) Run eval

```bash
python3 scripts/eval.py
```

The eval script is a deterministic offline quality gate. It indexes the checked-in fixture vault at `fixtures/eval/vault`, runs `fixtures/eval/queries.json`, and compares the report against `baselines/orderk-eval-baseline.json`. The gate fails on missing fixtures, zero-hit cases, top-1 regressions, or metric regressions in `recall_at_k`, `ndcg_at_k`, `mrr`, and mean latency. Advanced/dev-only overrides are available through `ORDERK_EVAL_VAULT`, `ORDERK_EVAL_QUERIES`, and `ORDERK_EVAL_BASELINE`; release runs should use the checked-in defaults.

The CLI prints JSON by default.

## Agent setup

This is the shortest path for an agent or automation:

1. Point `--vault` at the Obsidian vault.
2. Keep the SQLite DB inside the vault, usually under `.obsidian/orderk/orderk.sqlite`.
3. Use `siliconflow` as the embedding provider.
4. Set `BAAI/bge-m3` + `1024` unless you have a strong reason to change them.
5. Use `sqlite_vec` as the vector backend.
6. Consume the JSON output directly. Search responses include `route`, `routing`, per-result `score_breakdown`, `evidence`, `tags`, optional neighbor `context_chunks`, and optional Obsidian link evidence.
7. Use `--min-score`/`--threshold`, `--context-chunks`, and `--include-links` when an agent needs thicker evidence rather than more low-quality tails.
8. If the client supports MCP, use `orderk mcp` for read-only `search`/`status`/`health` tools instead of asking the agent to guess shell flags.
9. Use `orderk maintain --report-dir ...` as the agent-facing readiness/failure-ticket gate before release or scheduled checks.

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
npm pack --workspaces --dry-run
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

- `orderk-cli` is the Node wrapper around the native binary.
- `orderk-obsidian` is the desktop plugin package.
- Linux x64 one-click install is served from GitHub Releases.

## What orderk deliberately does not do

- chat
- agent orchestration
- note writing
- automatic summaries
- LLM reranking
- second-brain style lifecycle management

## License

MIT
