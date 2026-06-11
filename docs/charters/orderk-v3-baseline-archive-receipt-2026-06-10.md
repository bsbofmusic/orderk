# OrderK V3 Baseline Archive Receipt — 2026-06-10

> Status: archive receipt for the V3 release line before OrderK V4 / Jianling product-boundary work.  
> Scope: records the released V3 public surfaces and known caveats. This is not a new release and does not publish anything by itself.

## 1. Why this exists

OrderK V4 adds the built-in Jianling / Sword Spirit nightly Markdown memory compiler. Before V4 changes the product boundary, V3 must be preserved as the clean historical baseline: read-only Markdown search blade, full-vault-smart retrieval, default real Qwen3 reranker, MCP/CLI evidence tools, and no automatic note generation in active docs.

## 2. Verified V3 release surface

Collected from local repo, GitHub CLI, and npm registry on 2026-06-10.

### Git / repo

- Remote: `https://github.com/bsbofmusic/orderk.git`
- Current branch observed: `feat/orderk-v2-full-prd-20260604T120736Z`
- Current HEAD observed: `0db7b76 feat(release): ship orderk 0.1.17 Qwen reranker`
- Local tag at HEAD: `v0.1.17`

### GitHub Release

- Release tag: `v0.1.17`
- Release name: `orderk v0.1.17`
- Release URL: `https://github.com/bsbofmusic/orderk/releases/tag/v0.1.17`
- Published at: `2026-06-10T07:00:23Z`
- Asset: `orderk-v0.1.17-linux-x64`
- Asset size: `10,280,008` bytes
- Asset digest: `sha256:2d09addccc4824f3f586cd100b5609d24562ee770e3ae57ed318fbc30af7ab0b`
- Asset URL: `https://github.com/bsbofmusic/orderk/releases/download/v0.1.17/orderk-v0.1.17-linux-x64`

### npm

- Maintained npm package: `orderk-cli`
- Version: `0.1.17`
- Tarball: `https://registry.npmjs.org/orderk-cli/-/orderk-cli-0.1.17.tgz`
- Registry modified time: `2026-06-10T07:04:06.672Z`
- Repository metadata: `git+https://github.com/bsbofmusic/orderk.git`
- Important caveat: `npm view orderk` returns 404; checks must use `orderk-cli`, not bare `orderk`.

### Version files

- Root `Cargo.toml`: `version = "0.1.17"`
- Root `package.json`: `"version": "0.1.17"`

## 3. V3 capability baseline

V3 means the released read-only search baseline:

- Markdown-first local vault search;
- disposable/rebuildable SQLite index;
- full-vault-smart retrieval with source-tier and event-time signals;
- default real SiliconFlow `Qwen/Qwen3-Reranker-4B` reranker path;
- explicit `--reranker none` escape hatch for tests/migration;
- CLI/MCP evidence retrieval tools;
- no automatic note generation in active README/product boundary;
- no OrderK-owned nightly memory compiler yet.

## 4. Known caveat

GitHub Actions observation for `v0.1.17` showed:

- `release` workflow: success;
- `npm-publish` workflow: success;
- `ci` workflow on tag/branch: failure.

Therefore this receipt is a **V3 public release-surface archive**, not a claim that every remote CI workflow for `v0.1.17` was green. Any V4 release gate must not hide this: it should either fix the CI failure in a patch line or explicitly supersede it with a later CI-green release before declaring V4 release-ready.

## 5. V4 transition rule

V4 / Jianling work must treat this file as the V3 baseline receipt. V4 release notes must state that V4 changes the product boundary by adding OrderK-owned nightly Markdown memory compilation, while V3 remains the archived read-only search baseline.
