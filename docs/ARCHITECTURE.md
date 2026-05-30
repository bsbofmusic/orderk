# orderk architecture

## Boundary

orderk only does retrieval. It does not do chat, agent orchestration, note writing, automatic summaries, or LLM / cross-encoder reranking. It does expose an opt-in deterministic lexical reranker for bounded second-pass ranking.

It now also exposes explicit health and evaluation contracts:

- `status` for index snapshot + health state
- `health` for operational probe
- `doctor` for deeper probe + smoke query
- `eval` for black-box retrieval regression checks
- `maintain` for the cdper-style readiness/failure-ticket gate that nests health plus optional eval evidence and can persist a JSON report
- `capsule export` / `capsule inspect` for a Memvid-inspired portable manifest that binds a SQLite index to schema/profile/counts/size/checksum without copying notes or replacing the DB

## Runtime flow

```text
Obsidian thin wrapper
  -> orderk CLI
    -> orderk-core
      -> vault scanner
      -> markdown parser
      -> chunker
      -> embedding provider
      -> SQLite store
        -> files/chunks metadata
        -> FTS5 keyword index
        -> sqlite-vec vector index
      -> hybrid retriever
      -> score breakdown JSON
```

## Storage

The default store is one SQLite file. Capsule manifests are JSON receipts for that store: they record the DB path, combined DB/WAL/SHM byte size, aggregate SHA-256, per-file sidecar checksums, real SQLite settings schema/profile, and note/chunk/embedding counts. They are verification artifacts, not a replacement store or an import/restore format.

It contains:

- `files`
- `chunks`
- `chunk_embeddings`
- `fts_chunks`
- `vec_chunks`
- `settings`
- `feedback_events`

## Vector backend

Default backend: `sqlite-vec` virtual table.

Exact vector search exists as a fallback/test path, not the primary delivery path.

The index stores the active provider/model/dimension/backend in SQLite settings and refuses to search against a mismatched profile. That keeps agent-facing retrieval deterministic instead of silently mixing embeddings.

## Ranking

v1 uses lightweight score fusion and query-aware routing:

- keyword score from FTS5/BM25
- vector similarity from sqlite-vec
- reciprocal-rank fusion for keyword/vector candidate overlap
- query routing for short / path / tag queries
- explicit retrieval depth over authored Obsidian links: `--retrieval-depth 0` returns direct keyword/vector/route candidates; `--retrieval-depth 1` adds one-hop wikilink/backlink chunks as candidates with bounded `link_boost`
- the legacy `expand_links` field is accepted for backwards compatibility in MCP/JSON inputs but deprecated; prefer `retrieval_depth`
- path/tag/recency boosts
- optional chunk overlap at indexing time: `--chunk-overlap`
- deterministic lexical query expansion: `--query-expansion`
- JSON Lines output for pipeline consumers: `--json-lines`
- eval A/B for chunk overlap: `eval --ab-chunk-overlap`
- optional lexical reranker: `--reranker lexical|none`

Each search result also carries structured `score_breakdown`, `evidence` with `evidence_count` and per-result `retrieval_depth`, and tag metadata so agents can inspect why it surfaced. The response-level `routing.timings` reports keyword/vector/route/merge/link-expansion/enrichment stages, while `routing.retrieval_depth` states whether authored graph-depth recall was active.

### Pipeline stages → score_breakdown mapping

All scoring stages are additive and deterministic (no LLM, no cross-encoder, no runtime config except the bounded self-tuning optimizer which is opt-out).

```
Stage                          score_breakdown field            Configurable?
─────                          ──────────────────────            ────────────
1. Reciprocal-rank fusion      keyword_score, vector_score,      —
   (RRF + BM25 + vector)       route_score
2. Link expansion               link_score                       --retrieval-depth 1
   (one-hop wikilink/backlink) 
3. Temporal quality decay       temporal_quality_score           --freshness
4. Lexical reranker             rerank_bonus                     --reranker lexical (default on)
5. Bounded self-tuning          optimizer_adjustment             ORDERK_OPTIMIZER=off to disable;
   (text-only penalty)                                          orderk optimize set/reset for manual
6. Filter & truncate            min_score gate, top-N            --min-score, --limit
7. Enrich (context chunks,      evidence_count,                  --context-chunks
   link metadata)               retrieval_depth
```

Key invariants:
- Stages 1-4 always run in order; stage 5 (optimizer) only applies when ORDERK_OPTIMIZER is not disabled.
- The optimizer's `text_only_penalty` is clamped [0.65, 1.0], max 3 dynamic stopwords, auto-rollback after 3 consecutive adjustments if vector hit ratio drops 5%+.
- Every stage writes to `score_breakdown` — no hidden score manipulation.

No LLM or cross-encoder reranker is used; the only reranker path is the bounded deterministic lexical reranker.

## Feedback

DORMANT (v1): feedback_events are collected but not consumed in ranking or optimizer analysis. This is an intentional future-interface reservation, not a missing feature. The schema and `orderk feedback` command are in place for future self-tuning integration.
