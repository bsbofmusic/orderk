# orderk configuration

## CLI options

### Index

```bash
orderk index \
  --vault /path/to/vault \
  --db /path/to/vault/.obsidian/orderk/orderk.sqlite \
  --embedding-provider siliconflow \
  --embedding-dim 1024 \
  --embedding-model BAAI/bge-m3
```

### Search

```bash
orderk search \
  --db /path/to/orderk.sqlite \
  --query "naval leverage" \
  --limit 10 \
  --embedding-provider siliconflow \
  --embedding-dim 1024 \
  --embedding-model BAAI/bge-m3
```

## Providers

- `mock`: deterministic offline provider for testing.
- `siliconflow`: cloud provider path. Reads API key from `HERMES_SILICONFLOW_API_KEY` or `SILICONFLOW_API_KEY`.

Production default: `siliconflow` + `BAAI/bge-m3` + `1024`.
Use `mock` only for tests or offline smoke runs.

Recommended SiliconFlow model for production: `BAAI/bge-m3` with dimension `1024`.

## Obsidian plugin

The plugin is desktop-only because it shells out to a native binary.

Required settings:

- vault path
- CLI binary path, or `ORDERK_BIN`
- embedding provider/model/dim
- search limit
