# orderk troubleshooting

## Mental model

orderk is a scanner with a fault ticket printer:

1. Check whether the DB and provider profile are valid.
2. Index only changed files and deleted files.
3. Search only when embeddings/backend/profile are compatible.
4. Emit structured JSON evidence on failure.
5. Run fixture gates before release.

## Common symptoms

| Symptom | Likely cause | Fix | Verification |
|---|---|---|---|
| `SiliconFlow embedding API key is missing; set ORDERK_SILICONFLOW_API_KEY` | Production provider has no orderk-scoped key | Export `ORDERK_SILICONFLOW_API_KEY` | `orderk health ... --embedding-provider siliconflow` |
| `E_PROFILE_MISMATCH` | DB was built with a different provider/model/dim/backend | Use matching flags or rebuild the DB | `orderk status --db ...` then `orderk maintain ...` |
| `E_NO_EMBEDDINGS` / `needs_index` | DB exists but has no vectors yet | Run `orderk index` with the intended profile | `orderk status --db ...` |
| Search works by keyword but vector quality looks wrong | Wrong profile, stale index, or vector backend unavailable | Rebuild and run eval | `orderk eval --queries ...` |
| Obsidian plugin cannot run | Native CLI is missing | Set plugin binary path or `ORDERK_BIN` | `Orderk: Health Check` in Obsidian |
| npm package installed but CLI missing | Wrapper cannot resolve native binary | Check `ORDERK_BIN`, the npm vendor binary, GitHub release asset, or `orderk` on `PATH` | `orderk --version` |
| Startup indexing did not run | `indexOnStartup` disabled or vault path missing | Enable plugin setting and set vault path | Obsidian notice + `orderk status` |

## Evidence-first commands

```bash
orderk status --db /path/to/orderk.sqlite

orderk doctor \
  --db /path/to/orderk.sqlite \
  --vault /path/to/vault \
  --smoke-query "known phrase" \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024

orderk maintain \
  --db /path/to/orderk.sqlite \
  --vault /path/to/vault \
  --queries /path/to/eval-queries.json \
  --smoke-query "known phrase" \
  --limit 10 \
  --report-dir /tmp/orderk-reports \
  --embedding-provider siliconflow \
  --embedding-model BAAI/bge-m3 \
  --embedding-dim 1024
```

## Release failure triage

If `python3 scripts/release_gate.py` fails:

1. Read the failed command in `failed.cmd`.
2. Use `stdout_tail` / `stderr_tail` as evidence.
3. Fix only that failure.
4. Re-run the full release gate.

Do not publish from a partially green run.

## Non-goals to preserve

- No chat layer.
- No agent orchestrator.
- No note writing.
- No automatic summaries.
- No chat/generative answer layer; search reranking is retrieval-only and defaults to SiliconFlow `Qwen/Qwen3-Reranker-4B` (`--reranker none` only for tests/migrations).
- No background daemon or automatic polling by default.
