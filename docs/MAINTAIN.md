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

1. Rust tests.
2. Rust release build.
3. JSON contract fixture gate.
4. Smoke test.
5. Stress test with update/delete churn.
6. Eval fixture gate.
7. npm install.
8. workspace builds.
9. workspace tests.
10. npm pack dry-run.

It emits `orderk.release_gate.v1` JSON and fails on the first broken gate.

## Maintenance policy

- Production defaults stay `siliconflow + BAAI/bge-m3 + 1024 + sqlite_vec`.
- Mock embeddings are only for tests/offline smoke paths or explicit user flags.
- Profile mismatches are hard failures, not silent fallbacks.
- Feedback is recorded as evidence and future interface; v1 ranking does not consume feedback.
- Obsidian startup indexing is one incremental pass only when explicitly enabled; orderk does not start a background watcher or polling daemon.
