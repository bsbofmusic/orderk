# release notes

## Local verification

```bash
python3 scripts/release_gate.py
```

Equivalent expanded gate:

```bash
python3 -m unittest scripts/test_release_gate.py scripts/test_eval_gate.py scripts/test_feedback_to_eval.py
cargo test -p orderk-core --all-features query_options_
cargo test -p orderk-cli --all-features mcp_
cargo fmt --all -- --check
cargo clippy --workspace --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --all-features --release
python3 scripts/contract.py
python3 scripts/smoke.py
python3 scripts/stress.py
python3 scripts/eval.py
npm install
npm run build --workspaces --if-present
npm test --workspaces --if-present
npm pack --workspace orderk-cli --dry-run
```

`python3 scripts/release_gate.py` also runs internal version-consistency, secret-scan, package-cleanliness, Supermemory absorption regressions, release/eval/feedback-growth gate unit tests, resource-baseline, and eval-quality-baseline checks. Resource thresholds live in `baselines/orderk-resource-baseline.json`; deterministic eval thresholds live in `baselines/orderk-eval-baseline.json` and use `fixtures/eval/*`.

## Supermemory absorption surface

This release adds agent-facing retrieval controls without changing orderk's boundary:

- `orderk search --min-score` / `--threshold` to suppress low-score tails.
- `orderk search --context-chunks N` to include same-file neighbor chunks.
- `orderk search --include-links` to expose Obsidian wikilink/backlink evidence.
- `orderk search --expand-links 1` to optionally recall one-hop linked/backlinked chunks with bounded deterministic link evidence.
- `orderk search --query-expansion` to enable deterministic lexical query expansion.
- `orderk search --json-lines` to emit one result per line for pipe-friendly tooling.
- `orderk search --reranker lexical|none` to optionally apply a bounded deterministic lexical reranker after temporal-quality adjustment.
- `orderk index --chunk-overlap N` to preserve boundary context when chunk sizes cap out.
- `orderk eval --ab-chunk-overlap N` to compare overlap settings against the baseline eval run.
- `orderk mcp` as a thin read-only stdio MCP surface exposing only `search`, `status`, and `health` with standard `Content-Length` frames plus JSONL smoke compatibility.

These features return vault evidence only; they do not write notes, generate summaries, run chat, auto-save memories, or expose index mutation through MCP. The lexical reranker is deterministic and bounded; it is not an LLM or cross-encoder reranker.

## npm packages

- `orderk-cli`: the only maintained npm package; JavaScript wrapper for the native binary.
- `orderk-obsidian`: legacy/deprecated on npm; the Obsidian wrapper source remains in `packages/obsidian` for local/plugin builds.

Runtime installs resolve a stable native binary through `ORDERK_BIN`, a package-local vendor binary, or `orderk` on `PATH`; they do not depend on Cargo `target/` build artifacts.
The v0.1.10 npm one-click path targets Linux x64 first; other platforms can build from source and install/copy the resulting binary to a stable path.

## Obsidian artifact

```bash
npm run dist --workspace orderk-obsidian
```

The Obsidian wrapper build is source-only for GitHub/plugin packaging. Do not publish `orderk-obsidian` to npm.

Output:

```text
packages/obsidian/dist/main.js
packages/obsidian/dist/manifest.json
packages/obsidian/dist/styles.css
packages/obsidian/dist/versions.json
```
