# Resource benchmark

This report captures the resource envelope behind the README's lightweight claims.

The current retrieval workflow controls are opt-in and do not change the baseline claim that orderk stays a small, one-shot, read-only retrieval blade.

## Environment

- Host: Linux x64 maintainer machine
- Binary: `/home/agent/.local/bin/orderk`
- DB: `/home/agent/obsidian-vault/.obsidian/orderk/orderk.sqlite`
- Provider: `siliconflow`
- Model: `BAAI/bge-m3`
- Dimension: `1024`
- Vector backend: `sqlite_vec`

## Current live status

```json
{
  "ok": true,
  "notes": 2306,
  "chunks": 19081,
  "embeddings": 19081,
  "embedding_model": "BAAI/bge-m3",
  "embedding_dim": 1024,
  "vector_backend": "sqlite_vec"
}
```

## Snapshot

| Metric | Result |
|---|---:|
| Installed binary | 23,716,616 bytes (~22.6 MiB) |
| Release binary budget | <= 30 MiB |
| Live SQLite index | ~215 MiB |
| Measured VmRSS during live search | 9.2 MiB |
| Measured VmPeak during live search | 12.3 MiB |
| Resident orderk daemon count | 0 |
| Mock stress notes | 1,000 |
| Mock stress queries | 300 |
| Mock stress concurrency | 12 |
| Mock p50 / p95 | 72.4ms / 96.2ms |
| Initial mock index | 1,942ms |

## Commands

```bash
/home/agent/.local/bin/orderk status \
  --db /home/agent/obsidian-vault/.obsidian/orderk/orderk.sqlite \
  --json

python3 scripts/stress.py
```

RSS was sampled from `/proc/<pid>/status` while a live semantic search was running.

## What this proves

- orderk is a one-shot CLI with no resident daemon in normal runtime.
- The installed binary and measured RSS are small enough for local agent workflows.
- The live vault currently indexes 2,306 notes and 19,081 chunks / embeddings.

## What this does not prove

- It does not promise identical numbers on every vault.
- It does not include OS cache effects, provider latency distribution, or cross-platform builds.
- Live semantic search latency includes remote embedding-provider round trips.
