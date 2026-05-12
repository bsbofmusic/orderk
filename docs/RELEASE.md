# release notes

## Local verification

```bash
python3 scripts/release_gate.py
```

Equivalent expanded gate:

```bash
cargo test --workspace --all-features
cargo build --workspace --all-features --release
python3 scripts/contract.py
python3 scripts/smoke.py
python3 scripts/stress.py
python3 scripts/eval.py
npm install
npm run build --workspaces --if-present
npm test --workspaces --if-present
npm pack --workspaces --dry-run
```

## npm packages

- `orderk-cli`: JavaScript wrapper for the native binary.
- `orderk-obsidian`: Obsidian desktop plugin package.

Platform binary packages are intentionally left as a release-pipeline step; local development resolves `target/release/orderk` or `ORDERK_BIN`.
The v0.1.5 npm one-click path targets Linux x64 first; other platforms can build from source or point `ORDERK_BIN` at a local binary.

## Obsidian artifact

```bash
npm run dist --workspace orderk-obsidian
```

Output:

```text
packages/obsidian/dist/main.js
packages/obsidian/dist/manifest.json
packages/obsidian/dist/styles.css
packages/obsidian/dist/versions.json
```
