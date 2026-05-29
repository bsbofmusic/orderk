# orderk Supermemory absorption charter

Date: 2026-05-13

## Boundary

orderk stays a Rust-first headless Obsidian retrieval blade:

```text
Obsidian vault -> scan -> parse -> chunk -> embed -> SQLite FTS/vector -> JSON evidence
```

It does **not** become Supermemory:

- no chat UI;
- no note writing;
- no automatic memory extraction;
- no automatic forgetting / temporal lifecycle;
- no LLM query rewrite or LLM reranking in core;
- no AI SDK prompt injection middleware;
- no default agent-callable index mutation.

## Pre-flight rollback point

Implementation started from Git repo `/home/agent/orderk`.

- Base HEAD: `cc38300` (`chore: decouple runtime binary from build artifacts`).
- Rollback tag created before continuing implementation: `pre-steal-orderk-supermemory-20260513-105658`.
- Rollback command if this change must be abandoned before merge:

```bash
git reset --hard pre-steal-orderk-supermemory-20260513-105658
git clean -fd -- baselines fixtures scripts
```

Runtime DB/cache rollback: orderk indexes are disposable sidecar state. If schema v3 (`links_json`) causes trouble, rebuild the SQLite index from the vault with `orderk index`; Markdown notes are not modified by this change.

## Atomic source lesson breakdown

| Supermemory pattern | User pain it solves | Design choice observed | orderk absorption |
|---|---|---|---|
| result thickness controls | agents need either slim hits or richer evidence | API includes switches such as full docs / matching chunks / related memory | `--context-chunks N`, `--include-links`, existing routing/score JSON |
| threshold/min score | `limit` alone returns low-quality tails | search API exposes threshold knobs | `--min-score` / `--threshold` filters fused scores before return |
| neighboring chunks | exact hit may lack surrounding meaning | return matching chunks with contextual material | same-file before/after chunk evidence, no summarization |
| relation evidence | memory systems need relation context | graph/related memory surfaces | Obsidian-native wikilink/backlink evidence only |
| MCP recall surface | AI clients need a tool schema, not shell guessing | MCP server exposes recall tools | thin stdio MCP with only `search`, `status`, `health` |

## Absorb / Adapt / Reject

| Class | Item | Decision | Reason |
|---|---|---|---|
| Absorb | `min_score` / `threshold` | implemented | Makes quality/quantity tradeoff explicit and machine-readable. |
| Adapt | context chunks | implemented as `context_chunks` same-file neighbors | Uses indexed chunks only; no generated summaries. |
| Adapt | link evidence | implemented as Obsidian wikilinks/backlinks | Reuses vault-authored links, does not infer a memory graph. |
| Adapt | MCP recall | implemented as thin Rust stdio JSON-RPC/MCP subset | Gives agent-native schema while preserving read-only boundary. |
| Reject | save/forget/profile memory tools | rejected | Would turn orderk into a memory lifecycle system. |
| Reject | query rewrite / rerank | rejected | External agent may rewrite; orderk core remains deterministic evidence retrieval. |

## Schema / API changes

- DB schema version moves from `2` to `3`.
- `chunks.links_json TEXT NOT NULL DEFAULT '[]'` stores wikilinks found in chunk text.
- Migration is idempotent and backfills from existing chunk text.
- Search response adds optional `context_chunks` and optional `evidence.links`.
- `routing` adds `min_score`, `threshold_filtered`, `context_chunks`, and `include_links`.

## CLI / MCP surface

Search flags:

```bash
orderk search \
  --db /path/to/orderk.sqlite \
  --query "retrieval blade" \
  --limit 10 \
  --min-score 0.2 \
  --context-chunks 1 \
  --include-links
```

MCP server:

```bash
orderk mcp \
  --db /path/to/orderk.sqlite \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024 \
  --vector-backend sqlite_vec
```

MCP tools exposed: `search`, `status`, `health` only. The stdio server supports standard `Content-Length` frames and JSONL smoke compatibility. There is no MCP `index`, `feedback`, `maintain`, save, forget, write, or chat tool.

## Verification gates

Minimum gates for this absorption:

```bash
cargo test -p orderk-core --all-features query_options_
cargo test -p orderk-cli --all-features mcp_
cargo fmt --all -- --check
cargo clippy --workspace --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --all-features --release
python3 scripts/release_gate.py
npm run build --workspaces --if-present
npm test --workspaces --if-present
npm pack --workspaces --dry-run
```

Required smoke:

- CLI search using `--min-score`, `--context-chunks`, and `--include-links` against a fixture DB.
- MCP JSON-RPC `initialize` + `tools/list` confirms only read-only tools.
- MCP `tools/call search` returns structured JSON evidence.

## Product outcome standards

- Stable user entrypoints: `orderk search`, `orderk status`, `orderk health`, `orderk mcp`.
- Health/status remain machine-readable JSON.
- Release/eval/resource gates stay canonical for publishing.
- Runtime artifact remains the installed `orderk` binary; repo `target/` and npm `node_modules` are build scaffolding only.
- Failure evidence is stdout JSON / test output / release gate report.

## Go / no-go

Go only if:

- all Rust/npm/release gates pass;
- an independent subagent audit does not request boundary fixes;
- a built or installed artifact passes CLI and MCP smoke;
- no secrets, SQLite DBs, target artifacts, node_modules, or pycache files are committed.
