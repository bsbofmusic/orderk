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
| OrderK CLI is updated but MCP tools still behave like the old version | Existing `orderk mcp` process holds `/home/agent/.local/bin/orderk (deleted)` after binary replacement | Kill the stale MCP process and let the wrapper respawn, then re-run MCP status/health/search | `readlink /proc/<pid>/exe` is not `(deleted)` and `mcp_orderk_status/health/search` work |
| Jianling config looks right but live reflection is not called | Explicit hot switch is off or key-env pointer is unresolved | Set `ORDERK_JIANLING_LLM_ENABLED[_PROFILE]=1`; ensure `ORDERK_SWORD_LLM_API_KEY_ENV` points to the real key env, not a raw secret | `orderk jianling chat-smoke` and latest receipt show `provider_status=called_live` |
| Generated Jianling Markdown exists but search cannot find it | The generated file was not fed back into the active index | Run bounded `orderk index --path <generated.md>` against the active clean DB | `orderk search --query <run-id-or-title> --view index` returns the generated file |

## Evidence-first commands

```bash
orderk status --db /path/to/orderk.sqlite

orderk doctor \
  --db /path/to/orderk.sqlite \
  --vault /path/to/vault \
  --smoke-query "known phrase" \
  --embedding-provider siliconflow \
  --embedding-model Qwen/Qwen3-Embedding-4B \
  --embedding-dim 1024

orderk maintain \
  --db /path/to/orderk.sqlite \
  --vault /path/to/vault \
  --queries /path/to/eval-queries.json \
  --smoke-query "known phrase" \
  --limit 10 \
  --report-dir /tmp/orderk-reports \
  --embedding-provider siliconflow \
  --embedding-model Qwen/Qwen3-Embedding-4B \
  --embedding-dim 1024
```

## Release failure triage

If `python3 scripts/release_gate.py` fails:

1. Read the failed command in `failed.cmd`.
2. Use `stdout_tail` / `stderr_tail` as evidence.
3. Fix only that failure.
4. Re-run the full release gate.

Do not publish from a partially green run.

## Search/MCP non-goals to preserve

- No chat layer.
- No agent orchestrator.
- No note writing through search/MCP tools.
- No automatic summaries through search/MCP tools.
- No chat/generative answer layer; search reranking is retrieval-only and defaults to SiliconFlow `Qwen/Qwen3-Reranker-4B` (`--reranker none` only for tests/migrations).
- No background daemon or automatic polling in the default search path. `orderk jianling enable` is the explicit opt-in managed timer path for generated Markdown under `brain/`.
