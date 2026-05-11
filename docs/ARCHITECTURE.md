# orderk architecture

## Boundary

orderk only does retrieval. It does not do chat, agent orchestration, note writing, automatic summaries, or LLM reranking.

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

## Ranking

v1 uses lightweight score fusion:

- keyword score from FTS5/BM25
- vector similarity from sqlite-vec
- path/tag/recency boosts

No LLM reranker is used.

## Feedback

Feedback events are recorded but do not affect v1 ranking. This preserves a future self-evolution interface without making the first release heavy.
