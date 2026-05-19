# orderk maintenance contract

orderk is a headless retrieval blade, not a chat brain. The maintenance loop is intentionally mechanical:

```text
open -> health/doctor -> index/search only when profile is valid -> emit JSON evidence -> run fixture gates before release
```

## Product weapons

- **Jobs — one clean object:** one CLI, one Rust core, one SQLite file, one JSON contract. Do not add chat, note writing, daemon sprawl, or UI lifecycle bloat.
- **Musk — first principles:** retrieval quality must be measured by index freshness, profile validity, vector backend health, and deterministic eval results, not by vibes.
- **Naval — leverage:** every failure should become a reusable gate, report, or troubleshooting entry so the agent does not pay the same debugging cost twice.
- **Karpathy — keep the loop tight:** small changes, regression tests, no hidden assumptions, no unverified completion.

## Commands

### Quick readiness

```bash
orderk status --db /path/to/vault/.obsidian/orderk/orderk.sqlite
```

### Operational diagnosis

```bash
orderk doctor \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --vault /path/to/vault \
  --smoke-query "known phrase" \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024 \
  --vector-backend sqlite_vec
```

### Full maintenance gate

`maintain` combines health/doctor evidence with an optional eval file and can persist a failure/success report.

```bash
orderk maintain \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --vault /path/to/vault \
  --queries /path/to/eval-queries.json \
  --smoke-query "known phrase" \
  --limit 10 \
  --report-dir /path/to/reports \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024 \
  --vector-backend sqlite_vec
```

Output schema: `orderk.maintain.v1`.

The maintainer-facing query path now also supports the retrieval workflow controls surfaced in the CLI: `--chunk-overlap`, `--query-expansion`, `--json-lines`, `--reranker lexical|none`, and `eval --ab-chunk-overlap`. These are opt-in controls for recall quality and pipeline ergonomics, not new write paths.

### Continuous retrieval quality monitoring

`maintain --queries` can be wired into any monitoring system (cron, systemd timer, CI) to catch retrieval-quality regressions before users notice.

**1. Create eval queries for your vault**

Pick 4–8 stable, well-known files and write queries that should find them. Format (`orderk.eval_queries.v1`):

```json
{
  "schema_version": "orderk.eval_queries.v1",
  "queries": [
    {
      "id": "unique-id",
      "query": "natural language query that should find the target",
      "expected_paths": ["path/relative/to/vault.md"],
      "expected_phrases": ["distinctive phrase in target content"]
    }
  ]
}
```

A good eval query idempotently answers: _would a user searching for X find the right file at rank 1–3?_ The fixture at `fixtures/eval/queries.json` is a reproducible mock-provider baseline. For live vaults, create your own `live_queries.json` targeting real content.

**2. Schedule maintain with queries**

```bash
orderk maintain --db /path/to/db --queries /path/to/live_queries.json
```

On success the output includes:

```json
{
  "ok": true,
  "state": "ready",
  "eval": {
    "hits_at_k": 5,
    "mrr": 0.68,
    "mean_took_ms": 813.2
  },
  "checks": [
    {"component": "eval", "ok": true, "message": "eval gate passed"}
  ]
}
```

The `eval` block exposes `hits_at_k` (how many queries found their target in the top results), `mrr` (mean reciprocal rank), and per-query outcomes with `rank`, `top_path`, and `took_ms`. Set alert thresholds on `hits_at_k` and `mrr` — degradation means the index or ranking pipeline has drifted.

**3. Integration pattern (generic)**

Any scheduler can wrap this:

```text
run maintain --queries → parse JSON → if ok=false or eval hits < threshold → alert
```

The tool itself stays headless and one-shot; the scheduler is external (cron, systemd timer, CI pipeline, health-check endpoint). No daemon, no background polling inside orderk.

### Read-only MCP recall surface

For MCP-capable clients, run a thin stdio server:

```bash
orderk mcp \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024 \
  --vector-backend sqlite_vec
```

Only `search`, `status`, and `health` are exposed. The MCP server supports standard `Content-Length` stdio frames plus JSONL compatibility for smoke tests, opens the index through read-only search/status/health paths, and deliberately omits `index`, `maintain`, `feedback`, note-write, save, forget, summary, and chat tools.

Search can request thicker evidence with `min_score`/`threshold`, `context_chunks`, `include_links`, and the same metadata `filter` DSL as CLI search.

Important fields:

- `ok`: machine gate for agents.
- `state`: `ready`, `needs_index`, `degraded`, or `unhealthy`.
- `error_codes`: typed failure classes.
- `checks`: maintain-level gates such as eval pass/fail.
- `health`: nested `orderk.health.v1` report.
- `eval`: nested `orderk.eval.v1` report when `--queries` is provided.
- `report_path`: persisted JSON path when `--report-dir` is used.

## Release gate

Before publishing, run:

```bash
python3 scripts/release_gate.py
# or
npm run verify
```

The release gate runs:

1. Version consistency across Cargo, npm packages, Obsidian manifest, and `versions.json`.
2. Secret scan for common private key/token/API-key shapes.
3. Package cleanliness scan for build/runtime/private artifacts such as `target/`, `node_modules/`, vendor binaries, `.env`, logs, and SQLite files.
4. Supermemory absorption regression tests for search thresholds, neighbor chunks, Obsidian link evidence, and read-only MCP tool allowlisting.
5. Release/eval/feedback-growth gate unit tests: `scripts/test_release_gate.py`, `scripts/test_eval_gate.py`, and `scripts/test_feedback_to_eval.py`.
6. `cargo fmt --all -- --check`.
7. `cargo clippy --workspace --all-features -- -D warnings`.
8. Rust tests.
9. Rust release build.
10. Resource baseline gate from `baselines/orderk-resource-baseline.json`.
11. JSON contract fixture gate.
12. Smoke test.
13. Stress test with update/delete churn.
14. Eval fixture gate from `fixtures/eval/*` and `baselines/orderk-eval-baseline.json`.
15. npm install.
16. workspace builds.
17. workspace tests.
18. npm pack dry-run.

It emits `orderk.release_gate.v1` JSON and fails on the first broken gate.

## Maintenance policy

- Production defaults stay `siliconflow + BAAI/bge-m3 + 1024 + sqlite_vec`.
- Mock embeddings are only for tests/offline smoke paths or explicit user flags.
- Profile mismatches are hard failures, not silent fallbacks.
- Feedback is recorded as evidence and future interface; v1 ranking does not consume feedback.
- Obsidian startup indexing is one incremental pass only when explicitly enabled; orderk does not start a background watcher or polling daemon.
