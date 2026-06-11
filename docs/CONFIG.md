# orderk configuration

OrderK has configuration and logging surfaces, but not one monolithic always-on config/log daemon. Treat live state as layered and verify it with commands, not by reading one file.

## Runtime configuration surfaces

| Surface | Purpose | Truth check |
|---|---|---|
| CLI flags | Highest-priority runtime inputs: vault, DB, profile, provider/model/dim, reranker, `--path` | command invocation / JSON output |
| SQLite `settings` | Active index profile, embedding model/dim, vector backend, schema state | `orderk status --db <db> --json` |
| Environment provider slots | Embedding/reranker/LLM credential pointers and base URLs | `doctor`, `search`, `chat-smoke`; never print secret values |
| MCP wrapper | Agent-facing read-only surface and active DB binding | `mcp_orderk_status/health/search` plus process freshness |
| Jianling scheduler env | systemd-user profile env, flags, key-env pointers | `/home/agent/.config/orderk/<profile>.env` and `jianling doctor` |
| Jianling scheduler state | vault-local scheduler ownership/watermark/receipts | `.orderk/jianling/*` and `jianling status` |

Resolution rule: explicit CLI flags win over profile/env defaults. Search/index profile mismatches should fail closed rather than silently mixing embeddings.

## Logging and audit surfaces

| Surface | Purpose |
|---|---|
| CLI stdout JSON | Machine-readable success contract |
| CLI stderr JSON envelope | Typed failure contract |
| `.orderk/jianling/runs/<run-id>.json` | Jianling run receipt |
| `.orderk/jianling/runs/<run-id>.evidence.json.redacted` | redacted source/evidence pack |
| `.orderk/jianling/logs/<run-id>.log` | compact human run log |
| `.orderk/jianling/smoke/*.json` | live LLM smoke receipts |
| `journalctl --user -u orderk-jianling@<profile>.service` | systemd service runtime evidence |

A healthy config answer requires a live command/MCP check. File presence alone is not enough.

## CLI options

All CLI commands print JSON by default. The `--json` flag is accepted for explicit contract compatibility.

### Index

```bash
orderk index \
  --vault /path/to/vault \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --embedding-provider siliconflow \
  --embedding-dim 1024 \
  --embedding-model Qwen/Qwen3-Embedding-4B
```

### Search

```bash
orderk search \
  --db /path/to/orderk.sqlite \
  --query "naval leverage" \
  --limit 10 \
  --min-score 0.2 \
  --context-chunks 1 \
  --include-links \
  --expand-links 1 \
  --filter "confidence == 'high' && status == 'active'" \
  --embedding-provider siliconflow \
  --embedding-dim 1024 \
  --embedding-model Qwen/Qwen3-Embedding-4B
```

Optional search controls:

- `--min-score` / `--threshold`: filter low fused-score results after candidate ranking.
- `--context-chunks N`: include before/after same-file chunk evidence from the index.
- `--include-links`: include Obsidian wikilink/backlink evidence parsed from indexed Markdown.
- `--expand-links 1`: optionally expand recall one hop along indexed Obsidian wikilinks/backlinks. This adds deterministic `link_expansion` evidence and a small `score_breakdown.link_boost`; it is off by default and does not write notes.
- `--filter "tag == 'rust' && has_code == true && confidence == 'high'"`: apply the small whitelisted metadata filter DSL. Supported fields are `path`, `title`, `heading`, `tag`, `has_code`, `has_link`, `has_task_list`, `has_incomplete_tasks`, `confidence`, `status`, and `source_type`.
- `--reranker qwen`: run the default SiliconFlow `Qwen/Qwen3-Reranker-4B` model reranker. Missing reranker credentials fail closed with an explicit error.
- `--reranker none`: explicitly disable model reranking for testing/migration only.

### MCP read-only server

```bash
orderk mcp \
  --db /path/to/orderk.sqlite \
  --embedding-provider siliconflow \
  --embedding-dim 1024 \
  --embedding-model Qwen/Qwen3-Embedding-4B
```

MCP exposes only `search`, `status`, and `health`; it supports standard `Content-Length` stdio frames plus JSONL compatibility for smoke tests. It does not expose index, feedback, maintain, write, save, forget, or chat tools.

### Health / Doctor

```bash
orderk health \
  --db /path/to/orderk.sqlite \
  --vault /path/to/vault \
  --embedding-provider siliconflow \
  --embedding-dim 1024 \
  --embedding-model Qwen/Qwen3-Embedding-4B

orderk doctor \
  --db /path/to/orderk.sqlite \
  --vault /path/to/vault \
  --smoke-query "known phrase in your vault" \
  --embedding-provider siliconflow \
  --embedding-dim 1024 \
  --embedding-model Qwen/Qwen3-Embedding-4B
```

`status`, `health`, and `doctor` return `health_state` / `state`, `error_codes`, and structured `checks`. `doctor --smoke-query "..."` additionally runs a retrieval smoke probe; no arbitrary smoke query is injected by default.

### Maintain

```bash
orderk maintain \
  --db /path/to/orderk.sqlite \
  --vault /path/to/vault \
  --queries /path/to/eval-queries.json \
  --smoke-query "known phrase in your vault" \
  --limit 10 \
  --report-dir /tmp/orderk-reports \
  --embedding-provider siliconflow \
  --embedding-dim 1024 \
  --embedding-model Qwen/Qwen3-Embedding-4B
```

Maintain prints `orderk.maintain.v1` JSON. It nests `health` and optional `eval` evidence, writes a JSON report when `--report-dir` is set, and returns `state` plus typed `error_codes` for agent gating.

### Eval

```bash
orderk eval \
  --db /path/to/orderk.sqlite \
  --queries /path/to/eval-queries.json \
  --limit 10 \
  --embedding-provider siliconflow \
  --embedding-dim 1024 \
  --embedding-model Qwen/Qwen3-Embedding-4B
```

Eval prints a JSON report with `hits_at_k`, `top1_hits`, `zero_hit`, `recall_at_k`, `ndcg_at_k`, `mrr`, and mean latency, plus per-query matched ranks and result metadata. `python3 scripts/eval.py` is the checked-in offline quality gate: it indexes `fixtures/eval/vault`, runs `fixtures/eval/queries.json`, and validates the report against `baselines/orderk-eval-baseline.json`. Override those paths with `ORDERK_EVAL_VAULT`, `ORDERK_EVAL_QUERIES`, and `ORDERK_EVAL_BASELINE` for local experiments.

Eval query file schema:

```json
{
  "schema_version": "orderk.eval_queries.v1",
  "queries": [
    {
      "id": "example",
      "query": "known search phrase",
      "expected_paths": ["folder/note.md"]
    }
  ]
}
```

## Providers

- `mock`: deterministic offline provider for testing.
- `siliconflow`: cloud provider path. Reads API key from `ORDERK_SILICONFLOW_API_KEY`.

Production default: `siliconflow` + `Qwen/Qwen3-Embedding-4B` + `1024`. OpenAI-compatible providers use `ORDERK_OPENAI_API_KEY` / `ORDERK_EMBEDDING_API_KEY` plus optional `ORDERK_*_BASE_URL`.
Use `mock` only for tests or offline smoke runs.

Recommended SiliconFlow embedding model for production: `Qwen/Qwen3-Embedding-4B` with dimension `1024`. Recommended reranker: `Qwen/Qwen3-Reranker-4B`, enabled by default unless `--reranker none` is explicitly selected for tests/migrations.

## Obsidian plugin

The plugin is desktop-only because it shells out to a native binary.

Required settings:

- vault path
- CLI binary path, or `ORDERK_BIN`
- embedding provider/model/dim
- search limit
