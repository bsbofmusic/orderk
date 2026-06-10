# Token savings benchmark

orderk's token-efficiency story is not "summarize everything with an LLM".
It is progressive disclosure:

This report still centers `--view index` + `get`, but the current CLI also exposes `--json-lines` for pipelines and `--query-expansion` / `--reranker qwen|none` for retrieval shaping.

```text
search --view index -> choose candidates -> get selected chunks
```

In plain English: do not stuff the whole book into the model. Show it the table of contents first, then fetch the few pages it actually needs.

## Commands

Full search:

```bash
orderk search --db /home/agent/obsidian-vault/.obsidian/orderk/orderk.sqlite \
  --query "orderk retrieval blade" --limit 10 --json
```

Compact candidate cards:

```bash
orderk search --db /home/agent/obsidian-vault/.obsidian/orderk/orderk.sqlite \
  --query "orderk retrieval blade" --limit 10 --view index --json
```

Fetch selected chunks:

```bash
orderk get --db /home/agent/obsidian-vault/.obsidian/orderk/orderk.sqlite \
  --ids chk_3abf9d72d9bae40ac891d64c,chk_73762c4be14f4abeecb91d32,chk_27824a2fbb711473bc0cb43d \
  --detail full --json
```

## Results

| Query | Full search bytes | `--view index` bytes | `get` selected chunks bytes | Reduction full -> index | Same top file? |
|---|---:|---:|---:|---:|---|
| orderk retrieval blade | 22,830 | 13,400 | 2,996 | 41.3% | yes |
| Obsidian graph rules | 22,364 | 12,757 | 2,285 | 43.0% | yes |
| memory routing | 25,944 | 14,040 | not measured in this sample | 45.9% | yes |
| second brain protocol | 20,070 | 11,504 | not measured in this sample | 42.7% | yes |

## Read this correctly

`--view index` does not replace full evidence. It gives an agent cheap candidate cards first: `chunk_id`, title, path, line range, score.
Then `orderk get --ids` retrieves exact chunks only after the agent has chosen.

This is useful because it keeps the LLM from paying for every snippet before it knows which ones matter.

## What this proves

- Compact recall materially reduces response size on representative real-vault queries.
- The top file stayed the same in the measured samples.
- Two-stage recall is a measurable behavior, not just a README claim.

## What this does not prove

- It does not claim a fixed annual dollar saving.
- It does not map bytes to tokens exactly; tokenizer behavior varies by model.
- It does not remove the need to fetch full chunks when the answer requires exact evidence.
