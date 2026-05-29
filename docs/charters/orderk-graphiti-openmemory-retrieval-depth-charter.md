# orderk Graphiti / OpenMemory retrieval-depth charter

Date: 2026-05-17
Branch: `steal/graphiti-openmemory-orderk-20260517-164806`
Rollback tag: `pre-steal-orderk-graphiti-openmemory-20260517-164806`
Base commit: `cf5fd6f`
Source clones:

- Graphiti: `/tmp/orderk-steal-graphiti-openmemory/graphiti`, commit `9a2d6d0`
- OpenMemory: `/tmp/orderk-steal-graphiti-openmemory/OpenMemory`, commit `de39bcd`

## Target boundary

orderk remains a tiny, local, read-only retrieval blade for Obsidian Markdown vaults.

Must keep:

- Markdown files remain the source of truth.
- SQLite index remains disposable sidecar state.
- CLI / JSON / thin read-only MCP remain the main interface.
- Search returns grounded evidence; it does not generate, write, save, forget, or chat.
- No resident daemon, hosted memory system, graph database, LLM fact extractor, or automatic memory mutation.

Rollback:

```bash
git switch steal/graphiti-openmemory-orderk-20260517-164806
git reset --hard pre-steal-orderk-graphiti-openmemory-20260517-164806
```

Runtime artifact rollback if replaced:

```bash
cp /home/agent/.local/bin/orderk.<timestamp>.bak /home/agent/.local/bin/orderk
```

## Atomic source audit summary

### Graphiti lessons

| Atom | Finding | Source evidence | orderk decision |
|---|---|---|---|
| User pain | Knowledge changes over time; retrieval needs the right time/provenance instead of only latest text. | `graphiti_core/nodes.py` has `EpisodicNode.valid_at`, `source`, `source_description`, raw `content`; facts/edges carry `created_at`, `expired_at`, `valid_at`, `invalid_at`. Search filters expose valid/invalid/created/expired times. | Adapt later as optional frontmatter/mtime temporal facets. Do not add mutable fact invalidation now. |
| Design choice | Store episodes and facts in a graph, then search edges/nodes/episodes with BM25/vector/BFS/RRF/rerank recipes. | `graphiti_core/search/search_config.py`, `search_config_recipes.py`, `search.py`, `search_utils.py`. | Keep orderk's SQLite/Rust pipeline. Steal the concept of explicit retrieval depth and evidence path, not graph DB/search scopes. |
| Value lever | Multi-hop graph traversal adds memory depth when direct semantic or keyword match is shallow. | Graphiti BFS and node-distance search expand from origin/result nodes under depth limits. | Adapt as bounded Obsidian wikilink/backlink expansion. Depth is authored link evidence only. |
| Cost | Neo4j/FalkorDB/Kuzu/Neptune, OpenAI LLM/embedder/reranker, server/MCP ingestion, telemetry, saga summaries. | `graphiti_core/graphiti.py`, `server/graph_service`, `mcp_server`, cross-encoder clients. | Reject. Too heavy and violates read-only/no-daemon/no-chat boundary. |

### OpenMemory lessons

| Atom | Finding | Source evidence | orderk decision |
|---|---|---|---|
| User pain | Retrieval should favor important, recent, connected, and sector-relevant memory, not just raw vector similarity. | `memory/scoring.py`, `memory/hsg.py`, `memory/decay.py`, `ops/dynamics.py`. | Adapt into deterministic read-only score/evidence components derived from existing Obsidian/index signals. |
| Design choice | Salience, recency, coactivation/reinforcement, temporal graph, sectors, and explainable trace. | `core/constants.py`, `memory/hsg.py`, `temporal_graph/*`, `trace.py`. | Start with explicit `retrieval_depth` + depth evidence over existing links. Later add static sector/salience boosts only if eval proves value. |
| Value lever | Query-time trace makes retrieval behavior debuggable; coactivation adds depth beyond direct hit. | `trace.py`; HSG emits score pieces and graph/path signals. | Absorb trace contract: expose depth in `routing`, per-result `evidence`, and `score_breakdown`. |
| Cost | Query-hit reinforcement mutates salience/last_seen; reflection creates synthetic memories; server/connectors ingest external sources. | `memory/hsg.py`, `ops/dynamics.py`, `memory/reflect.py`, `server/routes/*`, integrations. | Reject all mutation/autonomy/hosted connector behavior. |

## Absorb / Adapt / Reject

### Absorb now

1. **Explicit retrieval-depth contract**
   - Rename the user-facing idea from generic link expansion to `retrieval_depth`.
   - `0` = direct keyword/vector/route candidates only.
   - `1` = include one-hop authored Obsidian wikilink/backlink candidates.
   - Keep `expand_links` as compatibility alias.

2. **Depth evidence trace**
   - Add `routing.retrieval_depth`.
   - Add `evidence.retrieval_depth` per result.
   - Direct results stay `0`; expanded results become `1`.
   - Keep existing `link_expansion` source and bounded `link_boost`.

3. **Deterministic ordering around expansion**
   - Sort candidate row IDs and break score ties by path/line/chunk ID.
   - This makes graph-depth retrieval repeatable for agents and tests.

### Adapt later, not this slice

1. Temporal facets: optional `--as-of`, frontmatter date parsing, `temporal_boost`.
2. Static salience/sector boosts: regex-only sectors and bounded deterministic scores.
3. Local diversity/MMR: only if eval shows repeated same-note crowding.
4. Evidence path objects: `seed -> link -> candidate`, only if JSON consumers need it.

### Reject

- Graph DB service stack.
- LLM extraction, contradiction resolution, fact invalidation, saga summaries.
- OpenAI/Gemini/cross-encoder reranking in the local core.
- Query-time writes: salience reinforcement, last_seen updates, memory consolidation.
- Hosted API/server/connectors/chat integrations.
- MCP save/forget/index/feedback/mutation tools.

## TODO / implementation plan

### P0: contract and tests

- Add `retrieval_depth` to `QueryOptions`, default `0`.
- Add `retrieval_depth` to `QueryRoutingEvidence`, default `0`.
- Add `retrieval_depth` to `SearchResultEvidence`, default `0`.
- Add tests that fail before implementation:
  - `query_options_retrieval_depth_zero_is_direct_only`.
  - `query_options_retrieval_depth_one_marks_expanded_evidence`.
  - `query_options_retrieval_depth_rejects_gt_one`.
  - `query_options_retrieval_depth_aliases_expand_links`.

### P1: core implementation

- Resolve `effective_retrieval_depth = max(options.retrieval_depth, options.expand_links)`.
- Reject depth > 1 with a clear error.
- Use effective depth to call existing link expansion.
- Mark expanded/touched candidates with `evidence.retrieval_depth = 1`.
- Keep `routing.expand_links` as alias field and add `routing.retrieval_depth`.
- Sort expansion row IDs and final result ties deterministically.

### P2: CLI/MCP/API surface

- Add CLI flag `--retrieval-depth <0|1>`.
- Keep `--expand-links <0|1>` as backward-compatible alias.
- Add MCP argument `retrieval_depth` with schema min `0`, max `1`, default `0`.
- Parse MCP `retrieval_depth` and combine with `expand_links`.
- Update TypeScript response types with optional depth fields.

### P3: docs and product framing

- Update README search options and agent setup text.
- Update architecture docs: retrieval depth is authored Obsidian graph expansion only.
- Keep non-goals visible: no automatic memory mutation / graph DB / daemon.

### P4: verification and release hygiene

- `cargo test -p orderk-core --all-features query_options_retrieval_depth`
- `cargo test -p orderk-core --all-features query_options_`
- `cargo test -p orderk-cli --all-features mcp_`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo build --workspace --all-features --release`
- `python3 scripts/contract.py`
- `python3 scripts/smoke.py`
- `python3 scripts/eval.py`
- `python3 scripts/release_gate.py`
- Replace stable artifact only after gates pass; backup old `/home/agent/.local/bin/orderk` first.
- Smoke stable artifact against the live Obsidian orderk DB.

## Go / no-go

Go if:

- The patch is a small Rust-native/API-contract slice.
- No DB migration is needed.
- Search remains read-only with no query-time write.
- `retrieval_depth` is capped at 1 and off by default.
- Tests prove depth 0 vs depth 1 behavior.
- Stable artifact smoke passes.

No-go if:

- Implementation needs new graph DB, daemon, fact extraction, LLM reranker, or query-time mutation.
- Depth > 1 becomes recursive graph traversal.
- The MCP surface exposes any mutation tool.
- Verification cannot be completed.
