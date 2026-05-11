# orderk configuration

## CLI options

All CLI commands print JSON by default. The `--json` flag is accepted for explicit contract compatibility.

### Index

```bash
orderk index \
  --vault /path/to/vault \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --embedding-provider siliconflow \
  --embedding-dim 1024 \
  --embedding-model BAAI/bge-m3
```

### Search

```bash
orderk search \
  --db /path/to/orderk.sqlite \
  --query "naval leverage" \
  --limit 10 \
  --embedding-provider siliconflow \
  --embedding-dim 1024 \
  --embedding-model BAAI/bge-m3
```

### Health / Doctor

```bash
orderk health \
  --db /path/to/orderk.sqlite \
  --vault /path/to/vault \
  --embedding-provider siliconflow \
  --embedding-dim 1024 \
  --embedding-model BAAI/bge-m3

orderk doctor \
  --db /path/to/orderk.sqlite \
  --vault /path/to/vault \
  --smoke-query "known phrase in your vault" \
  --embedding-provider siliconflow \
  --embedding-dim 1024 \
  --embedding-model BAAI/bge-m3
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
  --embedding-model BAAI/bge-m3
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
  --embedding-model BAAI/bge-m3
```

Eval prints a JSON report with `hits_at_k`, `top1_hits`, `zero_hit`, `recall_at_k`, `ndcg_at_k`, `mrr`, and mean latency, plus per-query matched ranks and result metadata.

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
- `siliconflow`: cloud provider path. Reads API key from `HERMES_SILICONFLOW_API_KEY` or `SILICONFLOW_API_KEY`.

Production default: `siliconflow` + `BAAI/bge-m3` + `1024`.
Use `mock` only for tests or offline smoke runs.

Recommended SiliconFlow model for production: `BAAI/bge-m3` with dimension `1024`.

## Obsidian plugin

The plugin is desktop-only because it shells out to a native binary.

Required settings:

- vault path
- CLI binary path, or `ORDERK_BIN`
- embedding provider/model/dim
- search limit
