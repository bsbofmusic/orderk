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
| `crates/orderk-cli` | native CLI entrypoint for index/search/status/feedback |
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

### 5) Inspect status

```bash
orderk status --db /path/to/vault/.obsidian/orderk/orderk.sqlite
```

### 6) Run health / doctor

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

### 7) Run eval

```bash
python3 scripts/eval.py
```

The eval report includes `recall_at_k`, `ndcg_at_k`, `mrr`, and per-case matched ranks.

The CLI prints JSON by default.

## Agent setup

This is the shortest path for an agent or automation:

1. Point `--vault` at the Obsidian vault.
2. Keep the SQLite DB inside the vault, usually under `.obsidian/orderk/orderk.sqlite`.
3. Use `siliconflow` as the embedding provider.
4. Set `BAAI/bge-m3` + `1024` unless you have a strong reason to change them.
5. Use `sqlite_vec` as the vector backend.
6. Consume the JSON output directly. Search responses include `route`, `routing`, per-result `score_breakdown`, `evidence`, and `tags`.

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
cargo test --workspace --all-features
cargo build --workspace --all-features --release
python3 scripts/smoke.py
python3 scripts/stress.py
python3 scripts/eval.py
npm install
npm run build --workspaces --if-present
npm test --workspaces --if-present
npm pack --workspaces --dry-run
```

If `rustfmt` and `clippy` are installed in your environment, run them too. They are part of the CI contract.

## Troubleshooting

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
