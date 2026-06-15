# orderk-cli

Node wrapper for the native `orderk` CLI.

`orderk` is a local retrieval blade for Obsidian Markdown vaults plus the optional Jianling Markdown memory compiler. The npm package exposes a JavaScript entrypoint and an `orderk` binary shim. On Linux x64, postinstall can download the matching native binary from the GitHub Release asset `orderk-v<VERSION>-linux-x64`; otherwise set `ORDERK_BIN` or install/build the native Rust binary yourself.

## Install

```bash
npm install -g orderk-cli
orderk --version
```

If your platform does not have a bundled release asset:

```bash
cargo install --git https://github.com/bsbofmusic/orderk orderk-cli --locked
export ORDERK_BIN=$(command -v orderk)
```

## Core commands

```bash
orderk index --vault /path/to/vault --db /path/to/vault/.obsidian/orderk/orderk.sqlite
orderk search --db /path/to/vault/.obsidian/orderk/orderk.sqlite --query "project notes" --view index
orderk get --db /path/to/vault/.obsidian/orderk/orderk.sqlite --ids chk_abc,chk_def
orderk mcp --db /path/to/vault/.obsidian/orderk/orderk.sqlite
```

Search/MCP are read-only retrieval surfaces. They do not expose note writing, save/forget, chat, or index mutation tools. Normal search uses the default Qwen reranker path; `--reranker none` is only an explicit test/migration escape hatch.

## Jianling

```bash
orderk jianling run --vault /path/to/vault --date 2026-06-10 --dry-run
orderk jianling self-check --vault /path/to/vault
orderk jianling chat-smoke --vault /path/to/vault
orderk jianling validate-run --vault /path/to/vault --run-id <run-id>
```

Jianling writes generated Markdown only under `brain/` and only after its Kanban writer/auditor/foreman hard gate accepts the run. Writer cards include `draft_markdown` and `draft_hash`; auditor cards check format, traceability, source-anchor coverage, and hash integrity; the foreman controls the final write. In the current v41 line, live MiniMax M3 reflection resolves as per-profile override, then global override, then default-on when a valid LLM chain/key-env pointer such as `ORDERK_SWORD_LLM_API_KEY_ENV` exists; explicit false/off overrides remain the kill switch, and provider failures fail closed.

## Environment

- `ORDERK_BIN`: explicit native binary path used by the wrapper.
- `ORDERK_SKIP_BINARY_DOWNLOAD=1`: skip postinstall download.
- `ORDERK_BINARY_URL`: override the release asset URL for postinstall.

## Security boundary

Do not publish vault contents, SQLite indexes, logs, or credentials. Keep API keys in environment variables; orderk docs and receipts should reference variable names, not key values.
