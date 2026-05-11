# orderk

orderk is a lightweight Rust search layer for Obsidian Markdown vaults.

It is intentionally **not** a chat app, agent, note writer, or second-brain operating system. It is a thin retrieval blade:

```text
Markdown vault -> chunks -> embeddings -> SQLite/FTS5/sqlite-vec -> hybrid JSON results
```

## Goals

- Faster/smaller/stabler retrieval path than heavyweight Khoj-style stacks for local Obsidian vaults.
- Keep Obsidian's existing workflow unchanged.
- Use keyword search and vector search together by default.
- Keep embedding providers pluggable. SiliconFlow + BGE-M3 is the intended cloud provider path; mock embeddings are available for offline tests.
- Keep feedback as an event log in v1, not an auto-learning ranking loop.

## Install

```bash
# Native CLI, global/system install
cargo install --path crates/orderk-cli --locked

# npm wrapper for users who want a JS entrypoint; the one-click path today is Linux x64 via GitHub release binary download
npm install -g orderk-cli

# Obsidian plugin build artifact
npm run dist --workspace orderk-obsidian
```

For cloud embeddings, set `HERMES_SILICONFLOW_API_KEY` or `SILICONFLOW_API_KEY` before indexing or searching.

## Quick start

```bash
cargo build --workspace --all-features --release

cargo run -p orderk-cli --bin orderk -- \
  index --vault tests/fixtures/sample-vault --db /tmp/orderk.sqlite \
  --embedding-provider mock --embedding-dim 16 --embedding-model mock-16 --json

cargo run -p orderk-cli --bin orderk -- \
  search --db /tmp/orderk.sqlite --query "sqlite vector search" \
  --embedding-provider mock --embedding-dim 16 --embedding-model mock-16 --json
```

## Workspace

```text
crates/orderk-core      Rust core: scanner/parser/chunker/store/retriever/ranker
crates/orderk-cli       Native CLI
packages/cli           npm wrapper for the native CLI
packages/obsidian      thin Obsidian desktop plugin wrapper
docs/                  architecture/config/release/security notes
tests/fixtures/        sample vault fixtures
scripts/smoke.py       local smoke test
```

## Verification

```bash
cargo test --workspace --all-features
cargo build --workspace --all-features --release
npm install
npm run build --workspaces --if-present
npm test --workspaces --if-present
npm pack --workspaces --dry-run
python3 scripts/smoke.py
python3 scripts/stress.py
```

`cargo fmt` and `cargo clippy` are part of the CI contract. They require a Rust toolchain with `rustfmt` and `clippy` installed.
