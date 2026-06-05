# Noteriv × orderk Headless Second-Brain PRD Receipt — 2026-06-05

## Change summary

Updated the orderk charter set with a new headless Noteriv × orderk second-brain completion PRD:

- `docs/charters/noteriv-orderk-headless-second-brain-prd.md`
- `docs/charters/README.md`

The PRD changes the V2 completion target from “released retrieval/digest slice” to **headless Noteriv second-brain completion**: memory library, knowledge library, and skill library must be fully integrated as first-class sources; automatic sync, scheduled health/eval, DR backup, rollback, restore rehearsal, and alerting must copy the mature Obsidian operating model.

## User correction absorbed

The user clarified that “接入 Noteriv” means:

- headless first; no UI/cockpit for this phase;
- integrate memory/knowledge/skill libraries completely;
- copy Obsidian’s mature automation shape: automatic update, scheduled jobs, disaster recovery, health checks, rollback, silent success, and abnormal alerts;
- explain why semantic graph did not come out and why Sword Spirit is weak;
- convert the PRD into a ✅/❌ todolist with no middle state.

## Files changed

1. `docs/charters/noteriv-orderk-headless-second-brain-prd.md`
   - Defines headless completion target.
   - Defines memory/knowledge/skill source adapters.
   - Defines Obsidian-style scheduled jobs and DR/rollback/alerting requirements.
   - Explains why graph did not come out in v0.1.15.
   - Explains why Sword Spirit is weak.
   - Adds ✅/❌ no-middle-state completion matrix and implementation todolist.

2. `docs/charters/README.md`
   - Adds charter index entry for the new PRD.

## Three-party audit

Round 1:

- Hermes native subagent: `REQUEST_CHANGES`
  - blockers: rollback missing as first-class ability; alerting not strongly verifiable; evidence pack missing rollback/alerting.
- SF DeepSeek V4 Pro: `REQUEST_CHANGES`
  - blockers: seed positive-edge corpus/gate missing; proposal supply guard missing; rollback/alerting not closed.
- MiMo `mimo-v2.5-pro`: `PASS`, with non-blocking warnings about alert channel, restore metrics, memory-source conflict priority, and Sword Spirit seed edge anxiety.

Round 2 after patch:

- Hermes native subagent: `PASS`
- SF DeepSeek V4 Pro: `PASS`
- MiMo `mimo-v2.5-pro`: `PASS`

Audit artifacts:

- `/tmp/noteriv-headless-prd-audit-deepseek.json`
- `/tmp/noteriv-headless-prd-audit-mimo.json`
- `/tmp/noteriv-headless-prd-audit-round2-deepseek.json`
- `/tmp/noteriv-headless-prd-audit-round2-mimo.json`
- Hermes subagent summaries are stored in the current Hermes session transcript.

## Verification

Planned verification before commit:

```bash
git diff --check
git diff --stat
git status --short --branch
```

No code/runtime behavior changed in this PRD-only patch.

## Rollback

To remove this PRD update before commit:

```bash
git restore docs/charters/noteriv-orderk-headless-second-brain-prd.md docs/charters/README.md docs/charters/noteriv-orderk-headless-second-brain-prd-receipt-2026-06-05.md
```

To revert after commit:

```bash
git revert <commit_sha>
```

## Future potential

This PRD is the source of truth for implementing headless Noteriv as a second-brain operating layer. Implementation should now proceed task-by-task from the ✅/❌ matrix, starting with source registry schema and adapters for memory, knowledge, and skill libraries.
