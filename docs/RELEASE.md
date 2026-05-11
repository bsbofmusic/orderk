# release notes

## Local verification

```bash
cargo test --workspace --all-features
cargo build --workspace --all-features --release
npm install
npm run build --workspaces --if-present
npm test --workspaces --if-present
npm pack --workspaces --dry-run
python3 scripts/smoke.py
```

## npm packages

- `@orderk/cli`: JavaScript wrapper for the native binary.
- `@orderk/obsidian`: Obsidian desktop plugin package.

Platform binary packages are intentionally left as a release-pipeline step; local development resolves `target/release/orderk` or `ORDERK_BIN`.

## Obsidian artifact

```bash
npm run dist --workspace @orderk/obsidian
```

Output:

```text
packages/obsidian/dist/main.js
packages/obsidian/dist/manifest.json
packages/obsidian/dist/styles.css
packages/obsidian/dist/versions.json
```
