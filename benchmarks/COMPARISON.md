# orderk comparison

This comparison is about product shape, not a claim that one tool replaces every other tool.

agentmemory, mem0, Letta/MemGPT, and similar projects are memory systems or agent runtimes.
orderk is a local retrieval blade for an existing Markdown / Obsidian vault. Its optional Jianling sidecar can compile selected raw/human-authored evidence into generated Markdown notes, but the search/MCP path remains read-only.

The current orderk shape also includes retrieval controls like `--chunk-overlap`, `--query-expansion`, `--json-lines`, `--reranker qwen|none`, and `eval --ab-chunk-overlap`.

## Quick comparison

| Dimension | orderk | agentmemory | mem0 / Letta class tools | Built-in note search |
|---|---|---|---|---|
| Type | Local retrieval blade | Memory engine + MCP/REST server | Memory API / agent runtime | Human-facing app search |
| Source of truth | Markdown vault | Memory DB / observations | Memory DB / runtime state | Markdown vault |
| Writes notes | Search/MCP: no. Jianling: opt-in generated Markdown with receipts/source anchors. | memory writes / observations | API/runtime state | manual only |
| Daemon/server | Search: no. Jianling: optional managed systemd-user timer. | Yes | often yes | app runtime only |
| Search | BM25 + vector + metadata + links | BM25 + vector + graph | vector / graph varies | keyword / app index |
| Agent surface | CLI + read-only MCP | MCP + REST + hooks | API / runtime | none or manual |
| Token control | `--view index` + `get --ids` | memory budget | integration-dependent | manual / context-heavy |
| Observability | JSON status/health/maintain | viewer / dashboard | dashboard varies | manual |
| Best for | grounded evidence retrieval | persistent agent memory | memory lifecycle / agent runtime | human note search |

## What orderk should borrow from memory systems

- Clear benchmark tables.
- Token-efficiency framing.
- Agent-facing retrieval APIs.
- Evidence-first output.
- Maintenance / eval gates.

## What orderk should not copy

- hidden automatic memory capture hooks
- opaque memory save/update/delete lifecycle outside Markdown
- auto-forget / decay as hidden mutation
- dashboard as primary UX
- hosted source-of-truth assumptions
- chat as the main interface
- always-on daemon as the default runtime

## Positioning sentence

If you want a memory operating system, use a memory system.
If you want a small, local retrieval blade for your Markdown vault, with optional auditable Markdown memory compilation, use orderk.
