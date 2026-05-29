# orderk Mem0 + Basic Memory + Cognee quality/evidence/trace steal charter

Date: 2026-05-18  
Branch: `steal/mem0-basicmemory-cognee-quality-evidence-trace-20260518-104706`  
Rollback tag: `pre-steal-quality-evidence-trace-20260518-104706`  
Base HEAD: `412789d`  
Stable artifact before work: `/home/agent/.local/bin/orderk` v0.1.6, 23591288 bytes

## Target boundary

orderk remains a local Rust/Obsidian retrieval blade:

- read-only search/get/MCP recall surfaces;
- no chat, answer generation, autonomous memory write/update/delete, daemon, cloud sync, or second-brain lifecycle;
- runtime uses stable installed artifact, not temporary source clones;
- SQLite sidecar is disposable runtime cache and public recall must not silently migrate/write it.

## Source audits

### 1. Mem0 — quality regression and feedback-to-eval discipline

Source commit: `mem0=79793b0`.

Evidence:

- `evaluation/evals.py` loads fixed result JSON, compares `question`, gold `answer`, predicted `response`, computes BLEU/F1/LLM judge, and writes structured evaluation metrics.
- `docs/core-concepts/memory-evaluation.mdx` frames memory quality as **accuracy + token cost + latency**, not accuracy alone; it explicitly warns that larger context can inflate benchmark scores without improving production memory quality.

Adapt for orderk:

- Keep deterministic offline eval gate and feedback-derived query fixtures.
- Add a cheap per-case evidence check: each eval case may require result snippets to contain one or more expected phrases. This catches “right file but wrong/local-empty chunk” regressions without LLM judges or live tokens.
- Keep `scripts/feedback_to_eval.py` one-shot/read-only; no feedback-influenced ranking or automatic learning loop.

Reject:

- LLM judge in the default release gate.
- Mem0 extraction/write/update/delete/cloud/profile/entity memory lifecycle.
- Token-heavy full-context benchmark tricks.

### 2. Basic Memory — stable evidence URI and context navigation

Source commit: `basic-memory=60ec672`.

Evidence:

- `src/basic_memory/api/v2/routers/resource_router.py` exposes `GET /resource/{entity_id}` using an external ID, validates the entity file path stays under the project root, checks file existence, then returns raw content.
- `src/basic_memory/mcp/clients/resource.py` centralizes `/v2/projects/{project_id}/resource/{entity_id}` path construction for MCP/resource consumers.
- `tests/api/v2/test_resource_router.py` verifies read-by-external-id, 404 for missing resources, and path traversal rejection.

Adapt for orderk:

- Add stable read-only evidence locators to search/get results: `orderk://chunk/<encoded_chunk_id>` plus `obsidian://open?path=<path>&line=<line_start>`.
- Keep existing `path`, `line_start`, `line_end`, and `context_chunks` navigation.
- URI is a locator only; it must not imply an HTTP server or write surface.

Reject:

- Resource create/update/delete endpoints.
- Basic Memory sync/watch/Observation/Entity lifecycle.
- Any path-based write or server dependency.

### 3. Cognee — explainable retrieval trace shape

Source commit: `cognee=8b0d687`.

Evidence:

- `cognee/modules/search/methods/get_retriever_output.py` separates retrieval stages: get objects, derive context, optionally derive completion, recording per-stage observability spans and counts.
- `cognee/modules/search/methods/get_search_type_retriever_instance.py` centralizes retriever selection and explicit parameters (`top_k`, `wide_search_top_k`, `neighborhood_depth`, etc.).
- `cognee/modules/retrieval/utils/query_state.py` tracks query rounds, context text, completion, and convergence for iterative retrieval.

Adapt for orderk:

- Add an opt-in `--explain` search flag that includes deterministic trace metadata already available in orderk: stages, candidate counts, returned count, timings, route, filter/min-score, retrieval depth, and per-result sources/ranks/score breakdown.
- Default output stays lean; explain is explicit.
- Trace is mechanical and evidence-oriented; no generated reasoning text.

Reject:

- Cognee graph DB, Cypher, graph completion, iterative CoT/completion, agents, workers, distributed queues, Neo4j, or LLM pipeline.

## Implementation plan

1. TDD RED:
   - eval phrase requirement fails until supported;
   - search/get result models fail until evidence URI fields exist;
   - CLI `search --explain` fails until query response can carry explain trace.
2. GREEN minimal implementation:
   - add optional `expected_phrases` to eval query cases and per-case result reporting;
   - add `evidence_uri` and `open_uri` to `SearchResult`, `SearchIndexEntry`, and `ChunkGetResult`;
   - add `QueryExplainTrace` to `QueryResponse`, gated by `QueryOptions.explain` / CLI `--explain` / MCP optional boolean.
3. Verification:
   - targeted Python/Rust tests for changed contracts;
   - fmt/clippy/workspace tests/build;
   - release gate;
   - stable artifact install + live read-only smoke;
   - independent review focused on read-only boundary, scope creep, and JSON contract.

## Go/no-go gates

Go only if all pass:

- no new write path in search/get/MCP recall;
- no daemon, server, graph DB, LLM judge, or memory lifecycle introduced;
- default search output remains compatible and lean except additive optional fields;
- eval gate still deterministic with mock embeddings;
- release gate passes;
- stable artifact smoke uses `/home/agent/.local/bin/orderk`;
- temporary `/tmp` source clones are cleaned after evidence is captured in this charter/reference.

Rollback:

```bash
git switch steal/mem0-basicmemory-cognee-quality-evidence-trace-20260518-104706
git reset --hard pre-steal-quality-evidence-trace-20260518-104706
cp /home/agent/.local/bin/backups/<latest-orderk-backup> /home/agent/.local/bin/orderk
```
