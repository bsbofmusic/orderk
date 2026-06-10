# orderk benchmark reports

This directory holds the longer-form evidence behind the README claims.

## Reports

- `RETRIEVAL.md` — accuracy and compact recall examples
- `RESOURCE.md` — binary size, RSS, daemon count, index footprint
- `TOKEN_SAVINGS.md` — full-context vs `--view index` vs `get` token-shape comparisons
- `COMPARISON.md` — orderk vs memory systems vs built-in note search
- These reports also reflect the current retrieval workflow controls: `--chunk-overlap`, `--query-expansion`, `--json-lines`, `--reranker qwen|none`, and `--ab-chunk-overlap`.

These files are meant to be auditable, not promotional.
Each report should list:
- the exact command used
- the environment / machine shape
- the raw or summarized result
- what the result does and does not prove
