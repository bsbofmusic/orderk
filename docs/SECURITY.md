# security

## Secrets

- API keys must come from environment variables or local Obsidian settings.
- Never print API key values.
- Never commit `.env`, SQLite indexes, or Obsidian vault private data.

## Provider key names

- `ORDERK_SILICONFLOW_API_KEY` for orderk SiliconFlow embeddings
- `ORDERK_OPENAI_API_KEY` / `ORDERK_EMBEDDING_API_KEY` for OpenAI-compatible orderk embeddings
- Hermes/SF provider keys such as `HERMES_SF_API_KEY` and `HERMES_SILICONFLOW_API_KEY` belong to Hermes chat/provider routing, not orderk production paths

## Plugin boundary

The Obsidian plugin shells out to the CLI. It does not parse or persist embedding API keys by default in this skeleton.

## Secret scan

```bash
git grep -nE 'OPENAI_API_KEY|ANTHROPIC_API_KEY|NPM_TOKEN|GITHUB_TOKEN|sk-[A-Za-z0-9]|ghp_[A-Za-z0-9]|BEGIN (RSA|OPENSSH|EC|DSA) PRIVATE KEY' -- . ':!package-lock.json' ':!Cargo.lock'
```
