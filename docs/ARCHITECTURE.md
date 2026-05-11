# orderk architecture

## Boundary

orderk only does retrieval. It does not do chat, agent orchestration, note writing, automatic summaries, or LLM reranking.

It now also exposes explicit health and evaluation contracts:

- `status` for index snapshot + health state
- `health` for operational probe
- `doctor` for deeper probe + smoke query
- `eval` for black-box retrieval regression checks

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

The default store is one SQLite file. It contains:

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
- path/tag/recency boosts

Each search result also carries structured `score_breakdown`, `evidence`, and tag metadata so agents can inspect why it surfaced.

No LLM reranker is used.

## Feedback

Feedback events are recorded but do not affect v1 ranking. This preserves a future self-evolution interface without making the first release heavy.
