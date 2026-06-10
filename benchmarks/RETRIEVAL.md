# Retrieval benchmark

This report captures orderk's own retrieval-quality evidence.
It is not a LongMemEval-S reproduction.

It now reflects the opt-in retrieval workflow controls shipped in the CLI: chunk overlap, deterministic query expansion, JSON Lines output, the default Qwen model reranker, and eval A/B for overlap.

## What was measured

Two layers matter here:

1. **Offline fixture eval** — deterministic local quality gate on the checked-in fixture vault.
2. **Live vault eval** — representative live queries against the maintainer vault.

## Commands

```bash
python3 scripts/eval.py
orderk maintain \
  --db /home/agent/obsidian-vault/.obsidian/orderk/orderk.sqlite \
  --queries /home/agent/orderk/fixtures/eval/live_queries.json
```

Representative compact-recall comparisons:

```bash
orderk search --db /home/agent/obsidian-vault/.obsidian/orderk/orderk.sqlite \
  --query "orderk retrieval blade" --limit 10 --json

orderk search --db /home/agent/obsidian-vault/.obsidian/orderk/orderk.sqlite \
  --query "orderk retrieval blade" --limit 10 --view index --json

orderk get --db /home/agent/obsidian-vault/.obsidian/orderk/orderk.sqlite \
  --ids chk_3abf9d72d9bae40ac891d64c,chk_73762c4be14f4abeecb91d32,chk_27824a2fbb711473bc0cb43d \
  --detail full --json
```

## Results

### Offline fixture eval

- Queries: 4
- Top-1 hits: 4/4
- Recall@k: 1.0
- nDCG: 1.0
- MRR: 1.0
- Mean query time: 1-3 ms in local fixture runs

### Live vault eval

- Queries: 5
- Hits@k: 5/5
- MRR: 0.6833332777
- Mean took: 813.2 ms

Representative live query outcomes:

| Query | Rank | Top path | Result |
|---|---:|---|---|
| orderk design query | 1 | `brain/systems/orderk-设计决策与定位.md` | hit |
| Obsidian graph query | 6 | `brain/concepts/Obsidian图谱规则.md` | hit |
| user preferences query | 1 | `brain/identity/茶老板偏好配置.md` | hit |
| second-brain protocol query | 1 | `brain/systems/第二大脑操作协议.md` | hit |
| skill version query | 4 | `wiki/sources/HermesSkillsMCP快照-9b2e54d2.md` | hit |

## Compact recall sample

For representative queries, `--view index` preserved the same top file while shrinking the output.

| Query | Full search bytes | `--view index` bytes | `get` bytes | Same top file? |
|---|---:|---:|---:|---|
| orderk retrieval blade | 22,830 | 13,400 | 2,996 | yes |
| Obsidian graph rules | 22,364 | 12,757 | 2,285 | yes |

Reduction on the above samples: **41.3%** and **43.0%**.

## What this proves

- orderk's checked-in fixture eval is stable and fully reproducible.
- `maintain --queries` can catch live quality drift with a small, explicit eval set.
- `--view index` + `get --ids` is a real two-stage recall pattern, not just a doc idea.

## What this does not prove

- It does not prove LongMemEval-S SOTA.
- It does not prove universal performance on arbitrary vaults.
- It does not remove provider/network latency from live semantic search.

For the benchmark-style README summary, see `README.md`.
