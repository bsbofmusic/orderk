# Noteriv × orderk Headless Second-Brain Completion PRD

> **⚠️ HISTORICAL — 此文档是旧 Noteriv × orderk 第二大脑方向的历史记录，不代表当前产品路线。保留仅供归档参考。当前 orderk 是 Obsidian 只读搜索刀，不做 Noteriv/第二大脑。**

> Status: Hermes + SF DeepSeek V4 Pro + MiMo audit round 2 PASS  
> Scope: headless only. No cockpit/UI in this PRD.  
> Rule: no middle state. Every requirement is either ✅ done or ❌ not done. Partial work is marked ❌ until the full acceptance gate passes.

## 1. One-line target

**Noteriv headless must become the Markdown-first second-brain operating layer that automatically ingests, indexes, digests, graphs, backs up, verifies, rolls back, restores, and audits the user's memory library, knowledge library, and skill library, with orderk as the retrieval/digest/semantic-graph core.**

Feynman version: Noteriv is the headless operating system, Markdown is the disk format, orderk is the self-built retrieval chip, Sword Spirit is the offline digest co-processor, and the cron/DR stack is the power supply + black box recorder. If any of those are missing, it is not a completed second brain.

User-facing completion definition: **the user can query any durable memory, knowledge note, or skill/playbook and receive ranked evidence with source paths; the system updates itself on schedule, detects regressions, backs itself up, can roll back bad generated state, and alerts only when something actually needs attention.**

## 2. What "彻底接入" means

"接入" does **not** mean "leave an adapter hook". It means copying the proven Obsidian operating model into Noteriv:

1. **Full source coverage**: memory library, knowledge library, and skill library are all first-class Noteriv sources.
2. **Automatic update**: changes are detected and indexed without manual command runs.
3. **Scheduled maintenance**: sync, health, recall eval, graph eval, backup, restore checks, and rollback rehearsals run by cron/systemd/Hermes cron.
4. **Disaster recovery**: every source category is covered by manifest, checksums, backup scope, restore rehearsal, rollback rehearsal, and retention policy.
5. **Silent success / noisy failure**: normal runs are silent; injected and real failures emit exactly one structured alert with gate id, severity, source ids, evidence path, and remediation hint.
6. **Rollback is first-class**: every source sync, index sync, digest, and graph update creates a last-known-good checkpoint. If any health/eval/DR gate fails, Noteriv can atomically roll `.noteriv/registry`, indexes, vectors, graph, proposals, and generated state back without mutating raw memory/knowledge/skill sources.
7. **No raw mutation**: Noteriv/orderk never rewrites raw memory/knowledge/skill sources without an explicit local approval flow.
8. **No UI dependency**: headless must be complete before cockpit/UI exists.

## 3. Source libraries to integrate

### 3.1 Memory library

**Definition:** durable user/profile/memory facts and Hindsight/Hermes memory surfaces.

Required source classes:

- Hermes `MEMORY.md` / `USER.md` injected facts.
- Hindsight long-term memory bank snapshots or export adapter.
- Session-search/history summaries when explicitly promoted.
- Preference registry and identity files.

Required behavior:

- Memory facts are scanned into Noteriv source registry.
- Each fact binds to source path or bank object id, hash, updated time, confidence, and scope.
- Search can distinguish user preference, environment fact, project convention, and temporary session note.
- Digest can propose consolidation or conflict markers, but cannot silently rewrite the canonical memory store.
- Conflict precedence is explicit: user profile/hard preference > Hindsight retained fact > skill reference > Obsidian/source mirror > session summary. Equal-rank conflicts become proposals, never silent overwrites.

### 3.2 Knowledge library

**Definition:** Markdown knowledge vaults: raw/wiki/source/concepts/entities/claims/decisions.

Required source classes:

- Obsidian-style vault mirror.
- Noteriv Markdown vault root.
- raw/source/concept/claim/decision files.
- Attachments metadata where available.

Required behavior:

- Same Obsidian-proven scan/index/query/health/backup shape is preserved.
- Source pages, concept pages, wikilinks, frontmatter, tags, and file hashes are indexed.
- orderk returns evidence paths/snippets/chunk ids, not vague summaries.
- Knowledge graph is rebuildable from Markdown + audit/sidecars.

### 3.3 Skill library

**Definition:** Hermes skill library and future Noteriv playbooks, including `SKILL.md`, references, templates, scripts, and assets.

Required source classes:

- `~/.hermes/skills/**/SKILL.md`
- skill `references/`, `templates/`, `scripts/`, `assets/`
- plugin-provided skills when exported into a stable source mirror
- Noteriv-native playbooks if added later

Required behavior:

- Skill metadata, triggers, pitfalls, commands, required env vars, and linked references are parsed.
- Skill search supports: by trigger, by task type, by pitfall, by command, by source system.
- Skill digest can propose routing updates or outdated-step warnings.
- Outdated-step detection uses evidence: command missing, file path missing, provider/model retired, linked reference contradicted by newer source, or user correction tagged to that skill.
- Skill changes must remain in the skill source of truth; Noteriv only indexes and proposes, not silently edits.

## 4. Headless architecture

```text
Noteriv Headless
  sources/
    memory/       # memory facts, Hindsight/Hermes exports, identity/preference registry
    knowledge/    # Markdown vault raw/wiki/source/concepts/claims/decisions
    skills/       # SKILL.md + references/templates/scripts/assets mirrors
  .noteriv/
    registry.jsonl        # source records: kind/path/id/hash/profile/scope
    chunks.sqlite         # chunk metadata and text index
    vectors.sqlite        # embedding vectors or vector ids
    graph.sqlite          # active semantic graph cache
    proposals.jsonl       # pending graph/wiki/skill/memory proposals
    rejected.jsonl        # rejected edges/patches/training negatives
    audit.jsonl           # every scan/digest/approve/backup/restore/rollback event
    checkpoints/          # last-known-good generated state snapshots
    profiles.json         # model/provider/dim/profile guard
    dr_manifest.json      # must_backup/should_backup/critical_files/checksums
    eval/                 # golden queries, recall eval, skill routing eval, graph eval
```

orderk owns scan/index/search/digest/graph/eval contracts. Noteriv owns source registry policy, scheduler, backup/restore/rollback, alerting, and eventual user-facing cockpit. In this PRD, cockpit is out of scope.

Scheduler decision for the headless phase: **Hermes cron is the orchestration surface, with script-only/no-agent jobs for watchdogs and evidence-producing agent jobs only for scheduled reasoning/audit tasks.** systemd timers may be used later, but are not required for this PRD.

## 5. Automatic update model copied from Obsidian

### 5.1 Scheduled jobs

| Job | Schedule | Success behavior | Failure behavior |
|---|---:|---|---|
| `noteriv-source-sync` | hourly | silent | alert with changed source counts and failing source ids |
| `noteriv-orderk-index-sync` | hourly after source sync | silent | alert on profile mismatch/provider failure/index corruption |
| `noteriv-digest-shadow` | daily | silent if no accepted proposal | alert if raw mutation, fallback, invalid proposal, proposal starvation, or budget breach |
| `noteriv-health` | daily | silent | alert with doctor JSON and gate id |
| `noteriv-recall-eval` | daily | silent if thresholds pass | alert on memory/knowledge/skill recall regression |
| `noteriv-graph-eval` | daily | silent if thresholds pass | alert on orphan ratio, zero accepted-edge streak, graph rebuild failure |
| `noteriv-dr-backup` | hourly | silent | alert on manifest gap, checksum mismatch, git/asset backup failure |
| `noteriv-restore-rehearsal` | weekly | silent | alert if restore cannot reconstruct registry/index/graph |
| `noteriv-rollback-rehearsal` | weekly and after failed upgrade/index/graph gates | silent | alert if last-known-good state cannot be restored or raw source hashes change |
| `noteriv-alert-contract-test` | daily in dry-run fixture | silent | alert if success emits output, failure emits no alert, or duplicate alerts appear |

### 5.2 Alert contract

- Success emits no stdout/stderr and no alert event.
- Failure emits exactly one structured alert per failure key.
- Alert sink for the headless phase: Hermes origin delivery or configured home channel; local copy under `.noteriv/alerts/*.jsonl`.
- Alert payload fields: `schema_version`, `job_id`, `gate_id`, `severity`, `source_ids`, `evidence_path`, `failure_summary`, `remediation_hint`, `dedupe_key`, `created_at`.
- Alert dedupe key: `job_id + gate_id + source_ids_hash + failure_kind`.

### 5.3 Obsidian mechanisms to copy exactly

- Read-only query mode for audit/benchmark.
- Source registry with hashes and timestamps.
- DR manifest covering all raw/source/generated critical paths.
- Git or release-asset backup with checksum verification.
- Health diagnostic checklist.
- Drift detection against design constitution.
- Silent cron success; non-empty output only on alert.
- Restore rehearsal, not just backup existence.
- Atomic last-known-good checkpoints before every write-producing headless job.
- Rollback rehearsal with failure injection, not only restore rehearsal.
- Raw/source mutation guards.
- Query/eval precision tests with exact expected source ids.

## 6. Semantic graph completion requirement

### 6.1 Why graph did not really come out in v0.1.15

Current v0.1.15 evidence proves the pipeline is alive but not graph-growth-complete:

- full-vault active scanned 3713 Markdown files.
- embedding called, reranker called, LLM called.
- raw unchanged true.
- neighbors sidecar exists.
- rejected sidecar exists.
- **accepted proposal / active semantic edge count is 0.**

Root causes to treat as PRD blockers:

1. **No accepted-edge fixture gate**: release can pass with rejected edges only.
2. **No seed positive-edge corpus**: Sword Spirit has no stable examples of good edges for this vault.
3. **No proposal supply guarantee**: a human precision gate requiring 50 proposals can deadlock if digest produces 0 proposals.
4. **LLM decision threshold is conservative**: bad edges are rejected, but good edges are not forced/proven.
5. **No graph store/explain command fully proving active edges**: graph is sidecar/proposal-shaped, not completed active graph.
6. **No human-approved seed edges**: without seed positive examples, Sword Spirit only learns to avoid pollution.
7. **No graph eval loop**: no daily measurement of accepted edge precision, orphan ratio, contradiction surfaced, approval rate.

### 6.2 What completed semantic graph means

A completed headless semantic graph must satisfy all of these:

- active edge store exists and is rebuildable.
- six relation types only: `supports`, `refines`, `contradicts`, `replaces`, `depends_on`, `part_of`.
- `orderk graph explain <id|path> --json` returns activated edges, evidence chunks, edge status, confidence, source paths.
- digest produces at least 10 known-good accepted edges on a controlled seed corpus.
- daily digest shadow produces either accepted/proposed edges or a `proposal_starvation` alert with evidence.
- rejected edges are retained as negative training/eval samples.
- graph boost cannot demote correct base top1.
- graph eval reports edge precision, orphan ratio, contradiction surfaced, approval rate.
- graph DR restore can rebuild active edges from Markdown + audit/sidecars.

Default graph thresholds for headless completion:

- controlled positive-edge fixture: accepted edge recall ≥ 0.70.
- manual proposal precision: ≥ 0.80 on 50 reviewed proposals.
- proposal supply: ≥ 50 reviewable proposals over the benchmark corpus, or explicit `proposal_starvation` FAIL.
- orphan ratio: ≤ 0.15.
- contradiction surfaced: ≥ 0.70 on conflict fixtures.
- graph-boost regression: 0 correct base top1 demotions.

## 7. Why Sword Spirit is weak today

Sword Spirit is weak **not because the idea is wrong**, but because the current shipped slice optimizes for safety/no-pollution before growth.

Observed weakness:

- full-vault active accepted edge/proposal count is 0.
- real-vault sample improvement is small: hit@3 +2, MRR +0.0127, top1 flat.
- no HS numeric parity run.
- no human-approved proposal precision set.

PRD-level causes:

1. **No positive edge benchmark**: it is tested for not breaking search, not for creating useful knowledge.
2. **No active graph feedback loop**: rejected edges exist, but accepted edges do not feed a measured graph improvement loop.
3. **No seeded gold graph**: Sword Spirit lacks examples of what a good edge looks like in this vault.
4. **No Noteriv memory/skill/knowledge unified source registry**: it only sees a vault-like corpus, not the full second-brain system.
5. **No scheduled digest/approval cycle**: a single active run is a smoke test, not a growing second brain.

Completed Sword Spirit requires: positive fixtures, accepted-edge gate, daily digest shadow, proposal supply guard, human approval queue, graph eval, and drift/DR checks.

## 8. Completion todo matrix

Rule: partial is ❌. No middle state.

### 8.1 Already done in v0.1.15

| Status | Item | Evidence |
|---|---|---|
| ✅ | GitHub Release published | `v0.1.15` release exists |
| ✅ | npm package published | `orderk-cli@0.1.15` clean install verified |
| ✅ | Clean-head release gate passes | `24/24 PASS` on commit `6a2b0b1` |
| ✅ | Query-time no-LLM default | subagent code audit confirmed |
| ✅ | Raw Markdown safety in full-vault active | `raw_unchanged=true` |
| ✅ | Embedding/reranker/LLM live path | full-vault active called all three |
| ✅ | 5-topic deterministic non-regression | base=5/5, sword=5/5 |
| ✅ | 50-sample real-vault no top1 regression | top1 33→33 |
| ✅ | 50-sample small quality lift | hit@3 37→39, MRR 0.735→0.7477 |
| ✅ | Proposal CLI exists | list/show/approve/reject shipped |
| ✅ | MCP write tools default disabled | v2 gate / audit confirms boundary |

### 8.2 Not done for completed Noteriv second brain

| Status | Item | Acceptance gate |
|---|---|---|
| ❌ | Noteriv memory library source registry | memory sources scanned, hashed, queryable, recoverable |
| ❌ | Noteriv knowledge library source registry | raw/wiki/source/concept/claim/decision roots registered and DR-covered |
| ❌ | Noteriv skill library source registry | SKILL.md + references/templates/scripts indexed and searchable by trigger/pitfall/command |
| ❌ | Automatic source sync | hourly silent success, alert on failing source ids |
| ❌ | Automatic orderk index sync for Noteriv | hourly sync with profile guard and exact health JSON |
| ❌ | Daily digest shadow job | scheduled, budgeted, raw-safe, proposal-only by default |
| ❌ | Daily recall eval across memory/knowledge/skills | exact expected hits and regression thresholds |
| ❌ | Daily graph eval | edge precision, orphan ratio, approval rate, contradiction surfaced |
| ❌ | Hourly DR backup | manifest coverage + checksum + remote/asset verification |
| ❌ | Weekly restore rehearsal | restore reconstructs registry/index/graph from backup |
| ❌ | Atomic rollback mechanism | failed source/index/digest/graph runs can restore last-known-good `.noteriv` state with checksum proof and `raw_unchanged=true` |
| ❌ | Alerting contract and silence proof | successful cron/Hermes jobs emit no output and no alert; injected failures emit exactly one structured alert with gate id and evidence path |
| ❌ | Active semantic graph store | active/rejected/superseded/conflict edges durable and rebuildable |
| ❌ | `orderk graph explain` | evidence-backed graph explanation JSON |
| ❌ | Seed positive-edge corpus | at least 10 known-good edges available for graph fixture |
| ❌ | Positive accepted-edge fixture | digest must generate known-good accepted edges in controlled corpus |
| ❌ | Proposal supply guard | benchmark corpus yields ≥ 50 reviewable proposals or hard FAIL |
| ❌ | Human proposal approval precision gate | 50 proposals reviewed, precision ≥ 0.80 |
| ❌ | HS-vs-Noteriv/orderk benchmark | strict path-hit and semantic fact-hit separated |
| ❌ | Memory consolidation proposals | conflicts/duplicates in memory facts generate proposals, not silent rewrites |
| ❌ | Skill routing eval | task → skill hit@k, stale-step detection, command/pitfall recall |
| ❌ | Disaster-recovery manifest for all libraries | memory/knowledge/skills all covered by `must_backup` / `critical_files` |
| ❌ | Design constitution + drift detector for Noteriv | drift gate blocks scheduler if core laws diverge |
| ❌ | Headless completion evidence pack | all ✅ gates mapped to command logs and artifacts |

## 9. Definition of done for headless Noteriv second brain

The headless second brain is done only when every ❌ above becomes ✅ and a final evidence pack contains:

1. `environment.json`: commit, version, dirty state, source roots.
2. `source_registry.json`: memory/knowledge/skill source counts, hashes, scopes.
3. `sync/`: hourly sync logs with silent-success proof.
4. `health/`: daily doctor JSON.
5. `recall_eval/`: memory/knowledge/skill recall metrics.
6. `graph_eval/`: edge precision, orphan ratio, approval rate, contradiction surfaced.
7. `digest/`: daily digest shadow runs, accepted/rejected/proposal counts.
8. `dr/`: manifest coverage, backup checksum, restore rehearsal.
9. `rollback/`: checkpoint creation logs, injected failure run, rollback execution proof, checksum comparison, and `raw_unchanged=true`.
10. `alerting/`: silent-success cron logs, injected-failure alert payloads, alert sink proof, dedupe key, severity, gate id, and evidence path.
11. `safety/`: raw unchanged, secret scan, MCP write boundary.
12. `hs_comparison/`: HS-vs-Noteriv/orderk strict path-hit and semantic fact-hit.
13. `task_matrix.json`: every PRD item mapped to ✅ evidence or ❌ failure.

## 10. Implementation todolist

Each item remains ❌ until its acceptance gate passes.

| Status | ID | Task |
|---|---|---|
| ❌ | T01 | Create Noteriv source registry schema for memory/knowledge/skill libraries |
| ❌ | T02 | Implement memory source adapter for Hermes/Hindsight memory exports |
| ❌ | T03 | Implement knowledge source adapter for Markdown vault raw/wiki/source/concepts/claims/decisions |
| ❌ | T04 | Implement skill source adapter for Hermes skills and linked files |
| ❌ | T05 | Add source hash/profile/scope tracking and drift detection |
| ❌ | T06 | Add scheduled `noteriv-source-sync` with silent success semantics |
| ❌ | T07 | Add scheduled `noteriv-orderk-index-sync` with profile guard |
| ❌ | T08 | Add daily `noteriv-digest-shadow` proposal-only job |
| ❌ | T09 | Add memory/knowledge/skill recall eval fixtures |
| ❌ | T10 | Add seed positive-edge corpus and graph eval fixtures |
| ❌ | T11 | Implement active edge store and rebuild path |
| ❌ | T12 | Implement `orderk graph explain --json` |
| ❌ | T13 | Add proposal supply guard and starvation alert |
| ❌ | T14 | Add proposal approval precision review workflow |
| ❌ | T15 | Add DR manifest covering all memory/knowledge/skill roots |
| ❌ | T16 | Add hourly backup with checksum verification |
| ❌ | T17 | Add weekly restore rehearsal |
| ❌ | T18 | Add atomic last-known-good checkpoints for registry/chunks/vectors/graph/proposals before every write-producing job |
| ❌ | T19 | Add rollback command and rollback rehearsal with failure injection and raw-hash proof |
| ❌ | T20 | Add alerting contract test: silent success emits nothing; injected failures emit exactly one structured alert |
| ❌ | T21 | Add Noteriv design constitution and drift detector |
| ❌ | T22 | Add HS-vs-Noteriv/orderk benchmark with path-hit/fact-hit split |
| ❌ | T23 | Add final task matrix generator |
| ❌ | T24 | Run final headless evidence pack and flip only passing rows to ✅ |

## 11. Non-goals for this headless phase

- ❌ No UI/cockpit requirement.
- ❌ No editor clone.
- ❌ No visual graph clone.
- ❌ No remote MCP self-authorization.
- ❌ No raw Markdown mutation by LLM.
- ❌ No HS control-plane clone.

Headless completion is allowed to be boring. It must be automatic, backed up, rollbackable, measurable, restorable, and quiet when healthy.
