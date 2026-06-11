# OrderK V4 Jianling / Sword Spirit PRD — Built-in Sleep Reflection & Markdown Memory Compiler

> Status: **P0/P1 slice implemented in `orderk-cli@0.1.18`; later LLM reflection phases remain gated**  
> Date: 2026-06-10  
> Owner intent: 茶老板提出“剑灵是睡后反思者”，不是外部 Hermes/agent cron，而是 OrderK 自身安装后、配置 LLM 后无感运转的内嵌夜间记忆整理机制。  
> Naming note: The new Jianling / Sword Spirit product generation is **OrderK V4**. V4 is built after the current V3 baseline, but it is not a patch label for V3. The old V3 release line must be archived clearly in npm, GitHub Releases, CHANGELOG, and repo docs before V4 changes the product boundary. Old `orderk-v2-sword-spirit-prd.md` remains historical and is not the active V4 design source.

---

## 1. 一句话定义

**OrderK Jianling 是 OrderK 内置的“睡后反思者”：它在夜间读取当天新增的原始 session / Markdown 事件，用 OrderK 自己的检索与 reranker 找回相关旧知识，再调用配置好的 LLM，把当天经验沉淀成可读、可审计、可 Git 管理的 Markdown；这些新 Markdown 随后被 OrderK 重新索引，形成向量与 reranker 更准的正反馈。**

费曼比喻：白天 OrderK 像图书管理员，帮你快速找书；晚上 Jianling 像闭馆后的馆员，把白天散落的纸条整理成日记、经验卡、决策卡、技能卡和概念页。第二天再搜索时，图书馆不是只多了一堆纸，而是真的多了整理好的书架。

---

## 2. Product Thesis

当前 OrderK 已经是轻量搜索刀：Markdown-first、可重建索引、混合检索、Qwen reranker、MCP/CLI 只读查询。它的短板不是“找不到”，而是**新经验进入后不会自动整理成更厚的知识网络**。如果所有 session 只是被索引为 chunk，未来能搜到原文，但难以形成稳定的原则、技能、决策和概念。

Jianling 的目标不是复制 Hindsight、Graphiti、EverOS 或 MemGPT，而是吸收它们最轻的机制：

- Reflexion：任务后语言反思；
- Generative Agents：重要性阈值 + 周期性反思；
- Voyager：可复用技能库；
- MemGPT / Letta：记忆分层；
- EverOS：Markdown 是 source of truth；
- Hindsight：retain/recall/reflect/consolidation 的接口心智，但拒绝黑盒 bank 和重 worker 系统。

最终产品判断：**OrderK V4 仍然继承 V3 的轻量搜索刀优势，但新增一个内置的 Markdown memory compiler，让知识库在夜间慢慢变厚。**

### 2.1 V3 baseline archive contract

V4 work must start by preserving the V3 line as a clean historical baseline. Current verified V3 release surface:

- GitHub repo: `https://github.com/bsbofmusic/orderk.git`.
- Latest GitHub Release observed: `v0.1.17`, name `orderk v0.1.17`, published `2026-06-10T07:00:23Z`, asset `orderk-v0.1.17-linux-x64`, size `10,280,008`, digest `sha256:2d09addccc4824f3f586cd100b5609d24562ee770e3ae57ed318fbc30af7ab0b`.
- npm maintained package: `orderk-cli`, version `0.1.17`, tarball `https://registry.npmjs.org/orderk-cli/-/orderk-cli-0.1.17.tgz`, repository `git+https://github.com/bsbofmusic/orderk.git`.
- Workspace version files for the P0/P1 implementation line say `0.1.18`.
- Important caveat: `npm view orderk` returns 404; release checks must use `orderk-cli`, not bare `orderk`.
- Current V3 capability baseline: read-only Markdown search blade, full-vault-smart source-tier retrieval, default real Qwen3 reranker, MCP/CLI evidence tools, no automatic note generation in active product docs.

V3 archive gate before V4 boundary change:

1. GitHub Release `v0.1.17` remains available with asset digest recorded in a V3 archive receipt.
2. npm `orderk-cli@0.1.17` registry metadata and tarball URL are recorded.
3. CHANGELOG records the V4 boundary change and V3 baseline archive without rewriting historical 0.1.17 claims.
4. `docs/charters/README.md` links this V4 PRD as implemented P0/P1 and keeps V2/Sword/Noteriv as historical.
5. V4 release notes must say: “V4 changes the product boundary by adding OrderK-owned nightly Markdown memory compilation; V3 remains the archived read-only search baseline.”
6. V4 CI/release gate must verify active version, GitHub release, npm package, and npx smoke for both: old V3 archive evidence and new V4 publish evidence.

---

### 2.2 Boundary transition / active docs state

This PRD records the V4 product-boundary change. The `0.1.18` implementation promotes the conservative P0/P1 slice: `orderk jianling` exists as an explicit Markdown compiler sidecar with deterministic digest generation, managed systemd-user timer files, receipts/evidence packs, validators, and safety gates. The search/MCP query path remains read-only; later LLM reflection is still behind future provider/eval gates.

Promotion rule:

1. Old `orderk-v2-sword-spirit-prd.md` remains historical and must not be used as the active design source.
2. `0.1.18` user-facing docs may say “OrderK has Jianling P0/P1” only for the deterministic digest/scheduler/receipt/validator slice.
3. User-facing docs must not claim autonomous LLM reflection is shipped until provider, budget, death-loop, and eval gates are implemented and pass.
4. Search/MCP docs must continue to state the query path is read-only; Jianling write behavior is explicit and separate.

### 2.3 Compatibility with current OrderK

Current OrderK surfaces include `index/search/get/status/doctor/mcp` and historical `digest` / `sword` code paths. Jianling introduces a new command namespace and config model rather than pretending existing commands already cover it.

Required compatibility decisions:

- New namespace: `orderk jianling ...`.
- Existing `sword` / historical digest commands are not reused as-is; any reuse must be hidden behind new schemas and migration tests.
- Config store: `.orderk/config.toml` inside the vault by default, with optional OS config fallback for global defaults.
- Resolution order: explicit CLI flags > profile in `.orderk/config.toml` > environment variables > built-in defaults.
- DB/vault binding must be explicit in config or CLI; scheduled runs must not guess a vault from cwd.
- P0 implementation target is Linux x64 with `systemd --user` timer plus one-shot worker; macOS LaunchAgent, Windows Task Scheduler, and server-loop scheduler are later phases.

---

## 3. Goals / Non-goals

### 3.1 Goals

1. **内置调度，不依赖外部 agent cron**  
   用户安装 OrderK 并配置 LLM 后，OrderK 自己管理 nightly reflection schedule。可以通过系统 timer/service 实现，但 timer 的创建、状态、日志、禁用、恢复均由 `orderk` 命令管理，不依赖 Hermes cron、外部 agent、手写 shell glue。

2. **Markdown-first consolidation**  
   Jianling 的所有长期产物必须是 `.md`；SQLite/vector/reranker cache 只是索引，可删可重建。

3. **睡后反思，不阻塞白天搜索**  
   默认 search/query path 不调用 LLM 反思；Jianling 在夜间或用户手动 `orderk jianling run` 时运行。

4. **正反馈闭环**  
   `raw session -> Jianling digest/reflection md -> orderk incremental index -> future search/rerank better -> next Jianling has better context`。

5. **主动但克制**  
   只沉淀高价值经验；普通闲聊、一次性进度、状态废话、无复用价值材料不写入长期层。

6. **可审计、可回滚**  
   每次 Jianling run 生成 run receipt、source refs、prompt/profile hash、文件 diff、secret scan 结果；必要时可按 run-id 回滚自动生成文件或撤销 patch。

7. **轻量资源边界**  
   常态无重 DB、无常驻大模型、无图数据库；夜间短时运行，云端 LLM；本地 RSS 和磁盘增长有硬预算。

### 3.2 Non-goals

- 不做 Hindsight 式 memory bank / operation queue / mental model 洪流；
- 不做 Graphiti 式重图数据库和实体关系全生命周期；
- 不把 LLM 生成答案放进默认 search path；
- 不让 LLM 原地改写 raw transcripts；
- 不自动修改用户手写的稳定原则/技能，除非策略明确允许且通过 patch gate；
- 不把 OrderK 变成聊天机器人或完整 agent runtime；
- 不依赖 Hermes cron、ChatGPT/Codex 外部 agent 或平台专属任务调度；
- 不因为“主动反思”每天生成大量垃圾 Markdown。

---

## 4. User Stories

### US1 — 无感夜间整理

作为用户，我希望安装 OrderK、配置 vault 和 LLM 后，不用每天手动触发，它会在夜间把当天 session 整理成 Markdown 日消化。

Acceptance:
- `orderk setup --enable-jianling` 后创建 orderk-managed schedule；
- `orderk jianling status` 能看到下一次运行时间、上次运行结果、LLM profile、写入数量；
- 正常成功静默，不打扰；失败写状态并可选通知。

### US2 — 经验越用越厚

作为用户，我希望反复出现的重要主题会从 daily digest 逐步升级成 reflection、lesson、skill、principle 或 wiki concept。

Acceptance:
- 同主题多次出现会提高 importance / repeat_count；
- Weekly run 能把多张 reflection 合并成 lesson；
- Monthly run 能把稳定 lesson 晋升为 skill/principle/wiki candidate；
- 低置信内容不会直接污染稳定层。

### US3 — 可追溯

作为用户，我希望任何沉淀结论都能追溯回原始 session、旧证据和生成模型。

Acceptance:
- 每张 generated md 包含 `source_sessions`、`source_chunks`、`generated_by`、`model_profile`；
- `orderk jianling explain <file>` 能显示来源、run-id、生成原因、是否自动写入/人工批准；
- 无 source refs 的结论不能自动进入 active 层。

### US4 — 安全克制

作为用户，我不希望剑灵把凭证、隐私、闲聊、状态废话、一次性流水写进长期层。

Acceptance:
- LLM 输入前和写入前都有 secret / PII / noise guard；
- secret scan 命中时该材料跳过或 redacted，run 状态为 degraded，不伪装成功；
- 每日新增文档数和字符数被 budget 限制。

---

## 5. Product Architecture

```text
                  ┌──────────────────────────┐
                  │ raw/transcripts/*.md      │
                  │ raw/articles/*.md         │
                  │ existing brain/wiki/*.md  │
                  └─────────────┬────────────┘
                                │
                                ▼
┌──────────────────────────────────────────────────────────┐
│ OrderK Core                                               │
│                                                          │
│  index/search/rerank                                     │
│  - FTS/vector/hybrid retrieval                           │
│  - Qwen reranker / configured reranker                   │
│  - source-tier metadata                                  │
│                                                          │
│  jianling subsystem                                      │
│  - scheduler manager                                     │
│  - run planner / watermark                               │
│  - evidence retriever                                    │
│  - LLM reflection compiler                               │
│  - markdown writer + patch/proposal writer               │
│  - audit / rollback / doctor                             │
└──────────────────────────────────────────────────────────┘
                                │
                                ▼
                  ┌──────────────────────────┐
                  │ generated Markdown        │
                  │ brain/daily               │
                  │ brain/reflections         │
                  │ brain/lessons             │
                  │ brain/decisions           │
                  │ brain/principles          │
                  │ wiki/concepts             │
                  └─────────────┬────────────┘
                                │ incremental index
                                ▼
                  ┌──────────────────────────┐
                  │ better OrderK search      │
                  │ better reranker context   │
                  │ better next Jianling run  │
                  └──────────────────────────┘
```

Jianling is not a separate product. It is an OrderK subsystem with its own commands, config, state, and OS integration.

---

## 6. Embedded Scheduling Model

### 6.1 Principle

“不是某个 agent 的 cron” does not mean no timer exists. It means **the timer belongs to OrderK**:

- configured by OrderK;
- visible through OrderK status/doctor;
- invokes OrderK binary, not a Hermes/Codex/ChatGPT script;
- uses OrderK config, profile, locks, audit, and failure semantics;
- can be disabled or removed with OrderK commands.

### 6.2 Supported runtime modes

OrderK should support two scheduling backends behind the same product surface:

1. **Managed OS timer backend** — default for CLI installs
   - Linux P0: `systemd --user` timer managed by OrderK. No Hermes/agent cron, no hand-written shell glue, no user-maintained crontab. A crontab compatibility backend, if ever added, is non-default, explicit opt-in, generated/disabled/statused only through `orderk jianling`, and cannot be counted as the P0 acceptance path;
   - macOS: LaunchAgent plist;
   - Windows: Task Scheduler;
   - Container/server: internal loop in `orderk serve` or explicit `orderk jianling worker`.

2. **Embedded server loop backend** — if user runs `orderk serve` / MCP server
   - A lightweight async scheduler lives inside the OrderK server process;
   - Uses the same lock/watermark/audit code as the one-shot timer;
   - Never runs concurrent jobs for the same vault/profile.

Both modes run the same command equivalent:

```bash
orderk jianling run --profile <profile> --scheduled
```

### 6.3 Linux P0 scheduler spec

P0 scheduler acceptance is intentionally narrow: Linux x64 + `systemd --user` + one-shot `orderk jianling run`. Cross-platform support is phased later.

Generated files:

```text
~/.config/systemd/user/orderk-jianling@<profile>.service
~/.config/systemd/user/orderk-jianling@<profile>.timer
```

Service contract:

```ini
[Service]
Type=oneshot
ExecStart=<absolute-orderk-bin> jianling run --profile <profile> --scheduled --vault <absolute-vault> --db <absolute-db>
EnvironmentFile=%h/.config/orderk/<profile>.env
WorkingDirectory=<absolute-vault>
```

Timer contract:

```ini
[Timer]
OnCalendar=*-*-* 03:30:00
Persistent=true
RandomizedDelaySec=300
```

Required behavior:

- `enable` writes/updates unit files, reloads user systemd, enables timer, and records scheduler backend in `.orderk/config.toml`.
- `disable` disables/removes only units owned by the current OrderK profile.
- `status` reports backend, unit names, binary path, vault, DB, next run, last run, last receipt, and whether the timer is active.
- `doctor` validates binary path exists, env file exists, LLM key env name exists without printing value, vault/DB paths exist, and no stale lock blocks the run.
- Missed runs use `Persistent=true`; after wake-up, only one catch-up run is allowed per profile.
- DST/timezone ambiguity is resolved by configured timezone; if unsupported by backend, receipt records `timezone_backend=system_local`.
- Logs go to journald plus `.orderk/jianling/logs/<run-id>.log` with secret redaction.

### 6.4 Installation flow

```bash
orderk init --vault ~/obsidian-vault
orderk config set llm.provider minimax
orderk config set llm.model M3
orderk config set llm.api_key_env MINIMAX_API_KEY
orderk jianling enable --schedule "03:30" --timezone Asia/Shanghai
orderk jianling doctor
```

If LLM is not configured, Jianling is disabled by default and `doctor` reports `LLM_MISSING`. Search/index still works.

### 6.5 Silent success / explicit failure

- Successful nightly run writes receipt and stays silent.
- Failure does not spam; it updates status and optional notification hook.
- Repeated failure N times can surface via `orderk doctor` or configured notifier.

---

## 7. Data Model: Markdown Memory Taxonomy

Jianling uses explicit path roots inside the vault. `brain/` is the generated-memory root; `wiki/concepts/` is a stable human-facing concept root and is proposal-patch only by default.

Default layout:

```text
brain/                         # generated_root: auto-created generated memory
  daily/                       # 每日消化
  weekly/                      # 周精炼
  monthly/                     # 月精炼
  yearly/                      # 年度宪法层
  reflections/                 # 任务后反思
  lessons/                     # 经验教训
  decisions/                   # 决策记录
  facts/                       # fact proposals / narrowly allowed generated facts
  open-loops/                  # 未闭环事项
wiki/
  concepts/                    # concept_root: stable concept pages; Jianling writes proposal patches only
.orderk/
  jianling/                    # control_root
    runs/*.json                # run receipts
    proposals/*.md             # 中置信 patch/proposal
    rejected/*.jsonl           # 低置信/噪音样本; also used by dedupe pre-checks
    topic_ledger.json          # topic-level repeat/dedupe ledger
    watermarks.json            # source cursor
    locks/                     # concurrency guards
```

### 7.1 Card types

| Type | Purpose | Auto-write policy |
|---|---|---|
| `daily_digest` | 当天结构化消化 | yes, one per day if meaningful input exists |
| `weekly_digest` | 7 天主题精炼 | yes, one per week |
| `monthly_digest` | 4 周合并、去重、晋升建议 | yes, one per month |
| `yearly_digest` | 长期系统/偏好/原则总结 | proposal-first |
| `reflection` | 某次任务后的经验反思 | yes if importance >= threshold, default status `active_generated` |
| `lesson` | 多次 reflection 合并的通用经验 | proposal-first or `active_generated` only after repeated primary evidence |
| `decision` | 架构/工具/边界决策 | proposal-first unless explicit user decision found |
| `principle` | 长期行为原则 | proposal-first, high bar |
| `skill_card` | 可复用 SOP | proposal-first; never edits external Hermes skills automatically |
| `fact` | 稳定事实 | proposal-first; auto `active_generated` only for explicit user-stated/config/project-state fact with direct quote |
| `concept` | wiki 概念页 | proposal-first for new page; patch proposal for existing page |
| `open_loop` | 未闭环问题 | yes, expires or resolves via future runs |

### 7.2 Standard frontmatter

```yaml
---
type: reflection               # daily_digest | weekly_digest | monthly_digest | yearly_digest | reflection | lesson | decision | principle | fact | concept | skill_card | open_loop
status: draft                  # draft | active_generated | active_user_approved | proposed | superseded | stale | rejected
created: 2026-06-10
updated: 2026-06-10
generated_by: orderk-jianling
jianling_version: 0.1
run_id: jianling-20260610-033000
model_profile: minimax-m3-standard
source_sessions:
  - raw/transcripts/hermes-sessions/2026/06/10/example.md
source_chunks:
  - orderk://chunk/chk_xxx
source_quotes:
  - path: raw/transcripts/hermes-sessions/2026/06/10/example.md
    line_range: [120, 155]
    quote_hash: sha256:...
    quote_text_excerpt_hash: sha256:...
    surrounding_hash: sha256:...
    source_file_hash: sha256:...
    captured_at: 2026-06-10T03:30:00+08:00
source_anchors:
  - path: raw/transcripts/hermes-sessions/2026/06/10/example.md
    anchor_strategy: line_range_then_quote_hash_then_surrounding_hash
    line_range: [120, 155]
    quote_hash: sha256:...
    surrounding_hash: sha256:...
fact_kind: null                # user_stated | external_claim | project_state | preference | config_state
valid_from: null
valid_until: null
verification_status: null      # single_source | multi_source | user_confirmed | stale
topics: [orderk, memory, consolidation]
importance: 8
confidence: 0.78
scope: default
supersedes: []
superseded_by: []
related:
  - "[[orderk搜索刀]]"
  - "[[第二大脑]]"
secret_scan: passed
---
```

Hard rule: no `source_sessions/source_chunks/source_quotes` means the card cannot be `active_generated` or `active_user_approved`.

Promotion status rules:

- `daily_digest` may be `active_generated` automatically when meaningful input exists.
- `reflection` may be `active_generated` only when it passes importance, evidence mix, dedupe, and template validation.
- `fact` defaults to `proposed`; it may be `active_generated` only when `fact_kind in {user_stated, project_state, config_state}` and there is a direct primary quote. `external_claim`, inferred facts, preferences, principles, skills, and profile changes are proposal-first.
- `decision`, `principle`, `skill_card`, `concept`, and user profile changes require `proposed` -> `active_user_approved` unless explicitly configured by local policy.
- `active_generated` means “generated synthesis with citations”, not raw truth. `active_user_approved` means user/human accepted it as stable knowledge.

Truth layer rules:

```text
raw_truth        = raw transcripts / imported original docs
human_truth      = user-authored stable notes
synthesis_layer  = generated_memory docs
```

Generated memory can summarize truth; it cannot define truth by itself.

Claim-level traceability rule:

- Every factual bullet, decision bullet, lesson, principle, and skill step must carry an inline source marker, e.g. `[src: raw/transcripts/...#L120-L155]` or `[src: orderk://chunk/chk_xxx]`.
- File-level frontmatter sources are necessary but not sufficient for active factual claims.
- Uncited factual claims may appear only in `draft` / `proposed` status.
- `orderk jianling explain <file>` must map each claim marker to `{path,line_range,chunk_id,quote_hash,role}` where role is `primary`, `retrieved_context`, or `prior_generated`.

### 7.3 Template registry and structured audit

Jianling templates are not hard-coded prose. They are versioned product artifacts stored under the vault-local OrderK control directory:

```text
.orderk/jianling/templates/
  daily_digest.v1.md
  reflection.v1.md
  lesson.v1.md
  decision.v1.md
  principle.v1.md
  skill_card.v1.md
  concept.v1.md
  open_loop.v1.md
  registry.json
.orderk/jianling/prompts/
  daily_reflection.v1.md
  weekly_merge.v1.md
  monthly_promotion.v1.md
  registry.json
.orderk/jianling/schemas/
  frontmatter.v1.json
  claim_refs.v1.json
  receipt.v1.json
  proposal.v1.json
```

`registry.json` must record:

```json
{
  "schema_version": "orderk.jianling.template.registry.v1",
  "active_templates": {
    "daily_digest": "daily_digest.v1.md",
    "reflection": "reflection.v1.md",
    "lesson": "lesson.v1.md"
  },
  "active_prompts": {
    "daily": "daily_reflection.v1.md",
    "weekly": "weekly_merge.v1.md"
  },
  "compatibility": {
    "frontmatter_schema": "frontmatter.v1.json",
    "claim_refs_schema": "claim_refs.v1.json",
    "receipt_schema": "receipt.v1.json",
    "can_read": ["frontmatter.v1", "claim_refs.v1", "receipt.v1"],
    "can_write": {"frontmatter": "frontmatter.v1", "claim_refs": "claim_refs.v1", "receipt": "receipt.v1"},
    "unknown_frontmatter_fields": "preserve",
    "unknown_required_sections": "validate_warning",
    "unknown_claim_ref_format": "validate_fail_for_active",
    "deprecated_after": null,
    "read_until": null,
    "migration_required_for_active": [],
    "min_orderk_version": "0.1.18",
    "max_orderk_version": null
  },
  "migrations": []
}
```

Structured audit validator:

```bash
orderk jianling validate-template --all
orderk jianling validate-file brain/daily/2026-06-10.md
orderk jianling validate-run --run-id <run-id>
```

Validator gates:

- frontmatter schema valid;
- required sections present for each card type;
- claim-level source refs resolvable;
- no secret/PII policy violations;
- status transition valid (`draft -> active_generated`, `draft -> proposed`, `proposed -> active_user_approved`, `active_generated -> superseded`, `active_user_approved -> superseded`);
- generated file type matches allowed auto-write policy;
- prompt/template/schema versions recorded in receipt;
- template drift detected if active registry hash differs from run receipt hash.

### 7.4 Reflection document content contract

A `reflection` card must answer a concrete “experience compiled into future advantage” question. It is not allowed to be a generic diary entry.

Required sections:

```md
# <short title>

## Trigger
- What happened? [src: ...]

## Preserved Event
- The concrete decision / correction / failure / success worth preserving. [src: ...]

## Root Cause / Mechanism
- Why it happened; must cite source or retrieved context. [src: ...]

## Future Reuse
- When this should be recalled next time.

## Actionable Rule
- One short rule, checklist item, or anti-pattern.

## Novelty / Delta
- What is new compared with prior generated or human-authored docs?

## Recall Cues
- Trigger phrases:
- Future situations:
- Anti-pattern names:

## Evidence Grade
- primary evidence:
- retrieved context:
- prior generated support:
- confidence reason:

## Validity / Expiry
- valid_until:
- stale_when:
- depends_on:

## Do Not Overgeneralize
- This does not imply:

## Conflict / Contradiction
- Conflicts with prior principle/skill/decision? If yes, downgrade to proposal.

## Links
- Related prior generated docs / raw sources / wiki concepts.
```

Anti-template rule: if Jianling cannot fill `Preserved Event`, `Future Reuse`, `Novelty / Delta`, and `Evidence Grade`, the reflection must be downgraded to `daily_digest` mention or rejected as chatter.

### 7.5 Prompt/template evolution and migration

Prompt and template changes are first-class migrations:

- Every prompt has `prompt_id`, `prompt_version`, `prompt_hash`, `model_family`, `input_contract`, and `output_schema`.
- Every template has `template_id`, `template_version`, `template_hash`, `frontmatter_schema`, and `required_sections`.
- Receipts store the prompt/template/schema hashes used for that run.
- A new prompt/template version must ship with fixture outputs and a migration note explaining what changes in generated Markdown.
- Old generated docs are never bulk-rewritten only because a prompt changed.
- If a schema migration is needed, it runs as `orderk jianling migrate --from vN --to vN+1 --dry-run` first and writes proposal patches, not silent edits.
- Prompt A/B tests must run on a fixed synthetic fixture and a real-vault copy; winning criteria are fewer false promotions, fewer duplicates, stable source refs, and no search regression.
- Reader compatibility is separate from writer compatibility: new OrderK must keep reading old `frontmatter_schema`, `claim_refs_schema`, and `receipt_schema` until `read_until`, even if it writes a newer schema.
- Unknown field policy is explicit: preserve unknown frontmatter fields, warn on unknown optional sections, fail active validation on unknown claim-ref formats.
- Prompt replay requires a redacted evidence pack: receipt stores `evidence_pack_hash` and `evidence_pack_path = .orderk/jianling/runs/<run-id>.evidence.json.redacted` so old prompt behavior can be audited.
- Migration risk classes must be declared: `non_breaking_template_change`, `schema_additive`, `schema_breaking`, `claim_ref_semantics_change`, `ranking_semantics_change`, `content_rewrite_forbidden`.
- Registry rollback is required: `orderk jianling registry rollback --to <registry-hash|run-id>` restores template/prompt/schema pointers without rewriting generated docs.
- Fixture golden sets live under `.orderk/jianling/fixtures/<fixture-set-version>/` with input hash, expected output hash, allowed nondeterministic fields, and pass/fail reason snapshots.

---

## 8. Daily / Weekly / Monthly / Yearly Consolidation

### 8.1 Daily run

Input:
- configured session globs, e.g. `raw/transcripts/hermes-sessions/YYYY/MM/DD/*.md`;
- any changed Markdown since last watermark, optionally source-tier filtered;
- previous 7/30-day generated digests for context;
- relevant existing wiki/brain docs retrieved through OrderK search.

Steps:

1. **Collect**: find new or changed source files since last watermark.
2. **Denoise**: preserve meaningful user/assistant/task content; drop tool logs, status chatter, duplicated context compaction, terminal noise, known generated files.
3. **Score**: calculate importance per event/topic.
4. **Retrieve**: use OrderK hybrid search + reranker to fetch relevant old docs, not raw grep only.
5. **Reflect**: call LLM with bounded evidence pack to produce structured candidate cards.
6. **Validate**: schema, source refs, secret scan, duplicate check, conflict check, size budget.
7. **Write**: create daily digest and validated low-risk `active_generated` reflections/open-loops; fact/decision/skill/principle/concept changes default to proposals unless the narrower fact exception passes.
8. **Index**: run incremental index on generated Markdown.
9. **Receipt**: write run JSON with counts, model, costs, paths, failures.

Daily digest template:

```md
---
type: daily_digest
status: active_generated
date: 2026-06-10
created: 2026-06-10T03:30:00+08:00
updated: 2026-06-10T03:30:00+08:00
generated_by: orderk-jianling
jianling_version: 0.1
run_id: jianling-20260610-033000
model_profile: minimax-m3-standard
source_sessions:
  - raw/transcripts/hermes-sessions/2026/06/10/example.md
source_chunks:
  - orderk://chunk/chk_xxx
source_quotes:
  - path: raw/transcripts/hermes-sessions/2026/06/10/example.md
    line_range: [120, 155]
    quote_hash: sha256:...
topics: [orderk, memory, consolidation]
importance: 7
confidence: 0.75
scope: default
supersedes: []
superseded_by: []
secret_scan: passed
pii_scan: passed
---

# 2026-06-10 Daily Digest

## 今日主线
- ... [src: raw/transcripts/hermes-sessions/2026/06/10/example.md#L120-L155]

## 事实变化
- ... [src: orderk://chunk/chk_xxx]

## 决策
- ... [src: raw/transcripts/hermes-sessions/2026/06/10/example.md#L180-L205]

## 经验 / 反思
- ... [src: orderk://chunk/chk_yyy]

## 待沉淀候选
- reflection: ... [src: raw/transcripts/...#L210-L240]
- skill: ... [src: raw/transcripts/...#L260-L310]
- principle: ... [src: raw/transcripts/...#L320-L350]

## Open loops
- ... [src: raw/transcripts/...#L360-L370]
```

### 8.2 Weekly run

Input:
- last 7 daily digests;
- new reflection/lesson/open-loop cards;
- relevant existing lessons/skills/principles/concepts via OrderK.

Purpose:
- merge repeated themes;
- resolve or carry open loops;
- promote repeated reflections into lessons;
- propose skill/principle/concept updates;
- mark duplicate low-value cards as superseded.

Output:
- `brain/weekly/YYYY-Www.md`;
- optional `brain/lessons/*.md`;
- proposal patches for `principles`, `skills`, `wiki/concepts`.

### 8.3 Monthly run

Input:
- 4 weekly digests;
- lessons/decisions/principle proposals;
- user-approved / rejected proposal history.

Purpose:
- deduplicate and compress;
- convert stable lessons into skill/principle candidates;
- update concept pages through proposal patches;
- detect stale or contradictory old material.

Output:
- `brain/monthly/YYYY-MM.md`;
- promoted `skill_card/principle/concept` proposals;
- stale/supersede recommendations.

### 8.4 Yearly run

Input:
- 12 monthly digests;
- active principles, decisions, skill cards, concept pages;
- user profile/fact cards.

Purpose:
- produce long-lived system overview;
- refine stable user preferences and project boundaries;
- create yearly architecture/history summary;
- avoid daily noise entirely.

Output:
- `brain/yearly/YYYY.md`;
- proposal patches to stable principles and concept pages.

---

## 9. Importance / Promotion Algorithm

Jianling must not save everything. A rule-based gate runs before LLM and after LLM.

```text
importance =
  user_emotion      # 0..3 explicit correction, praise, frustration, strong preference
+ failure_cost      # 0..3 failed task, rollback, CI red, data risk
+ repeat_count      # 0..3 same theme in 7/30/90 days
+ future_reuse      # 0..3 can become SOP/skill/checklist
+ decision_weight   # 0..3 changes architecture/tool boundary/config
+ evidence_strength # 0..3 real command/file/source proof
- chatter_noise     # 0..4 status chatter, one-off progress, idle conversation
```

Each candidate stores scoring provenance:

```json
{
  "score_source": "rule|llm|hybrid",
  "score_confidence": 0.0,
  "importance_breakdown": {
    "user_emotion": {"score": 0, "reason": "", "evidence_ref": ""},
    "failure_cost": {"score": 0, "reason": "", "evidence_ref": ""},
    "repeat_count": {"score": 0, "reason": "", "evidence_ref": ""},
    "future_reuse": {"score": 0, "reason": "", "evidence_ref": ""},
    "decision_weight": {"score": 0, "reason": "", "evidence_ref": ""},
    "evidence_strength": {"score": 0, "reason": "", "evidence_ref": ""},
    "chatter_noise": {"score": 0, "reason": "", "evidence_ref": ""}
  }
}
```

LLM may propose scores, but rule gates own hard blocks and final promotion policy.

Hard blocks are not negative scores; they stop promotion before LLM or before write:

```text
secret_risk: block_or_redact
pii_policy_block: block_or_local_only
missing_source_refs: draft_only
schema_invalid: reject
raw_mutation_attempt: reject
```

Calibration fixture requirement: each release must include synthetic sessions with known expected classes: no-write, digest-only, reflection, narrow fact exception, fact proposal, decision proposal, skill proposal, secret-blocked, PII-local-only, duplicate-suppressed, generated-only-loop, reflection-about-reflection, and weak-meta-memory-chatter.

Default thresholds:

```text
0-3   no long-term write; raw stays searchable
4-6   daily digest mention only
7-9   reflection / lesson candidate
10+   decision / skill / principle / concept proposal
```

Daily caps:

```text
max_daily_digest_files: 1
max_auto_reflections_per_day: 5
max_auto_generated_facts_per_day: 0        # facts are proposal-first except narrow direct-quote exceptions
max_narrow_auto_fact_exceptions_per_day: 2
max_proposals_per_day: 8
max_rejected_records_per_day: 50
max_generated_chars_per_day: 30_000
max_llm_calls_per_daily_run: 8
```

Weekly/monthly/yearly caps:

```text
max_weekly_digest_files: 1
max_weekly_lessons: 8
max_weekly_proposals: 12
max_weekly_generated_chars: 40_000
max_monthly_digest_files: 1
max_monthly_promotions: 12
max_monthly_proposals: 20
max_monthly_generated_chars: 60_000
max_yearly_digest_files: 1
max_yearly_principle_or_profile_proposals: 20
max_yearly_generated_chars: 80_000
```

Longitudinal anti-bloat caps:

```text
max_14_day_generated_files: 120
max_30_day_generated_chars: 1_000_000
duplicate_title_or_topic_ratio_max: 0.15
proposal_retention_days: 90
rejected_retention_days: 30
```

If budget is exceeded, Jianling writes a degraded receipt and queues remaining candidates; it does not force-run or silently drop without receipt.

### 9.1 Dedupe and death-loop suppression

Generated Markdown must improve retrieval, but it must not become self-feeding noise. The core rule is:

> Generated docs may be indexed and retrieved as context; generated docs alone cannot justify a new active reflection.

Dedupe keys:

```text
source_fingerprint = hash(primary_source_paths + primary_quote_hashes)
topic_fingerprint = hash(normalized_title + topic_cluster + action_rule)
action_rule_fingerprint = hash(normalized_actionable_rule)
semantic_fingerprint = embedding-near-duplicate over generated body
candidate_id = hash(primary_sources + normalized_claims + action_rule_fingerprint)
lineage = prior_generated_doc_ids + primary_raw_doc_ids
```

Evidence mix gates:

```text
active reflection: primary_raw_quote_count >= 1
active reflection: primary_evidence_char_ratio >= 0.30
lesson proposal: primary_or_human_evidence_char_ratio >= 0.25
principle/skill proposal: independent_primary_or_human_sources >= 2
prior_generated_context_ratio <= 0.50
prior_generated_max_evidence_pack_ratio <= 0.30 when raw/human context is available
generated_lineage_depth > 2 without new primary evidence: reject
```

Topic ledger:

```json
{
  "topic_cluster": "...",
  "action_rule_fingerprint": "sha256:...",
  "first_seen": "...",
  "last_seen": "...",
  "primary_evidence_count": 3,
  "generated_card_count": 1,
  "last_active_reflection_at": "...",
  "cooldown_until": "...",
  "last_action": "linked_existing|weekly_candidate|rejected_duplicate"
}
```

Write suppression rules:

- If candidate evidence is only `prior_generated` and has no `primary` raw/session/source quote, reject as `generated_only_loop`.
- If candidate is “today I reflected on yesterday’s reflection” with no new external/raw event, reject as `reflection_about_reflection`.
- If `topic_fingerprint` matches an active reflection/lesson in the last 7 days, do not create another reflection; update the daily digest mention or link the prior card.
- If `action_rule_fingerprint` matches an active card in the cooldown window, update ledger/link existing rather than create a same-rule card.
- If `candidate_id` is in recent rejected fingerprints within 30 days, skip before LLM as `skip_pre_llm_rejected_recent`.
- If `semantic_fingerprint` similarity is above 0.92 against an active card, create a supersede/update proposal only when there is new primary evidence; otherwise reject as duplicate.
- Repeated themes increase `repeat_count` and update `.orderk/jianling/topic_ledger.json`, but promotion happens in weekly/monthly consolidation, not by creating one daily reflection per day.
- At most one active reflection per `{topic_cluster, week}` unless `failure_cost >= 3` or `decision_weight >= 3`.
- Memory/process meta topics (`memory`, `reflection`, `orderk`, `jianling`, `process`) require a concrete raw event: external engineering event, bug, release, explicit user decision, rollback, or measurable failure cost. “今天想了想记忆系统” is digest-only at most.
- Open loops can be carried forward, but unchanged open-loop carry text does not count as new evidence.

Retriever role policy:

```text
primary          = raw transcript/source material from the target time window
retrieved_context = older raw/wiki/brain evidence fetched by OrderK
prior_generated = previous Jianling-generated docs
```

Promotion rules:

- `daily_digest` may cite `prior_generated` for continuity.
- `reflection`, `lesson`, `decision`, `principle`, `skill_card`, and `concept` require at least one `primary` or older human-authored source quote.
- Stable-layer proposals must show what is new versus prior generated docs.
- Generated context quarantine: docs generated in the current run cannot be used by a later stage in the same run to promote stable knowledge. They become low-weight continuity context only from the next scheduled daily run.

Death-loop fixture requirement:

- Fixture A: day 1 generates a reflection; day 2 only contains the previous reflection and no new raw event -> no new reflection.
- Fixture B: three days repeat the same weak meta-comment “今天整理了记忆” -> daily mention at most, no active reflection/lesson.
- Fixture C: same topic repeats with new concrete failures over 3 days -> one weekly lesson proposal, not three duplicate reflections.
- Fixture D: prior generated doc is retrieved by reranker but has no primary evidence -> candidate rejected with reason `generated_only_loop`.

---

## 10. LLM Integration

### 10.1 Provider abstraction

Jianling uses the same provider profile system as OrderK embedding/reranking, extended for LLM reflection.

Example config:

```toml
[llm.default]
provider = "minimax"
model = "M3"
api_key_env = "MINIMAX_API_KEY"
timeout_seconds = 90
max_retries = 2
fallback_policy = "fail_closed"   # fail_closed | proposal_only | skip

[jianling]
enabled = true
schedule = "03:30"
timezone = "Asia/Shanghai"
llm_profile = "default"
budget_profile = "standard"
[jianling.paths]
generated_root = "brain"
concept_root = "wiki/concepts"
control_root = ".orderk/jianling"
raw_roots = ["raw/transcripts", "raw/articles"]
session_globs = ["raw/transcripts/hermes-sessions/**/*.md"]
```

MiniMax M3 is a good default candidate because the task is long-text reflection and structured Markdown generation, not latency-critical query reranking.

Provider implementation contract:

```rust
trait LlmProvider {
    fn complete_structured(
        &self,
        prompt: PromptBundle,
        schema: JsonSchema,
        budget: LlmBudget,
    ) -> Result<StructuredCompletion, LlmFailure>;
}
```

Required profile fields:

```text
provider, model, base_url?, api_key_env, timeout_seconds, max_retries,
max_input_tokens, max_output_tokens, structured_mode, cloud_consent,
fallback_policy, prompt_version, prompt_hash
```

Failure enum must map provider-specific errors into stable Jianling statuses: auth, timeout, rate_limit, schema_invalid, content_policy, guard_blocked, budget_exceeded, provider_5xx, network_error. Token/cost accounting is best-effort but status/fallback accounting is mandatory.

### 10.2 Prompt contract

Jianling prompts must be versioned. LLM output must be structured JSON or Markdown with strict frontmatter, then validated.

Prompt principles:

1. Use only provided source and retrieved evidence.
2. Never invent source refs.
3. Prefer fewer, higher-value cards.
4. Preserve decisions and corrections exactly.
5. Redact secrets and do not store raw credentials.
6. When uncertain, write proposal/draft, not active principle.
7. Existing stable docs are not overwritten directly.

### 10.3 Provider failure semantics

Provider status must distinguish:

```text
not_configured
not_called_budget_skip
called_success
called_schema_invalid
called_timeout
called_provider_error
called_secret_guard_blocked
called_pii_guard_blocked
called_policy_guard_blocked
called_budget_exceeded
called_partial_proposal_only
```

No fallback may be disguised as success. `success` requires provider success, schema validation success, guard success, write success, index smoke success, and receipt write success. Any degraded sub-status must make the run `degraded` or `failed`, never `success`.

---

## 11. Write Governance

### 11.1 Raw immutability

`raw/` is immutable from Jianling’s perspective. Jianling may read raw transcripts and cite them, but it may not rewrite, summarize-in-place, delete, or redact raw source. Redaction only applies to LLM input bundle and generated output.

### 11.2 Candidate / active separation

- New daily digest can be `active_generated` automatically.
- Reflection/open-loop can be `active_generated` only if source refs, evidence mix, dedupe, template validation, and confidence pass.
- Fact changes default to proposal-first; narrow automatic fact exceptions require direct primary quote, allowed `fact_kind`, validity fields, and verification status.
- Decision/principle/skill/concept/profile changes default to proposal-first and require user approval for `active_user_approved`.
- Existing stable pages are patched through proposal unless user config explicitly enables auto-apply for that type.

Proposal location:

```text
.orderk/jianling/proposals/<run-id>-<slug>.md
```

Proposal includes:
- target file;
- proposed diff or new file body;
- source refs;
- LLM profile;
- validation result;
- why it should be promoted.

### 11.3 Conflict handling

If new evidence contradicts old principle/decision/fact:

- do not delete or overwrite;
- create `conflict` section in daily/weekly digest;
- generate a `decision` or `principle` patch proposal with `supersedes` fields;
- require approval for stable-layer changes unless high-confidence config allows auto-supersede.

### 11.4 Locks, watermarks, and transaction order

Lock key is `{vault_id, profile, mode}` where mode is `daily|weekly|monthly|yearly|manual`. Lock file lives under `.orderk/jianling/locks/` and uses atomic create-new semantics with `{pid, host, started_at, ttl_seconds, binary_path, run_id}`. Stale lock recovery requires either TTL expiry or explicit `orderk jianling unlock --run-id`.

Watermark state lives in `.orderk/jianling/watermarks.json` and records source path, content hash, mtime, last_processed_run, and last_status. Generated Markdown under `brain/` / `wiki/` must be excluded from raw-source collection unless the phase explicitly reads prior generated digests.

Transaction order:

```text
collect -> plan -> pre_llm_guard -> retrieve -> llm -> validate -> pre_write_guard -> write/propose -> index smoke -> receipt -> advance watermark
```

Rules:

- Watermark advances only after receipt is written.
- If write succeeds but index fails, watermark may advance with `status=degraded_index_failed` only if file_ops are fully recorded.
- If provider/guard/schema fails before write, no watermark advance unless the source is marked `skipped_by_policy` with reason.
- Crash recovery replays from receipt + file_ops; it never guesses by scanning generated file names alone.

---

## 12. Index Feedback Loop

After writing generated Markdown, Jianling triggers incremental indexing:

```text
write md -> update file metadata -> chunk -> embed -> update FTS/vector -> search smoke -> receipt
```

The new docs must become retrievable by OrderK. A run is not successful unless:

- written files exist;
- index update succeeds or is explicitly queued;
- at least one smoke query can retrieve the daily digest by title/date/topic;
- doctor marks generated files as indexed under the same embedding profile.

If indexing fails, generated Markdown remains a generated artifact on disk, but it is not considered searchable synthesis until reindex succeeds; run status is `degraded_index_failed`. Raw/human-authored evidence remains the truth layer.

MVP index integration contract:

- P0/P1 may reuse existing whole-vault hash-based indexing instead of a targeted `index --paths` implementation.
- Scheduled runs must pass explicit `--vault` and `--db` from config; cwd must not decide the active vault.
- Receipt records embedding profile fingerprint, chunk options, index command/method, files_seen, files_changed, chunks_changed, and index duration.
- Future optimization may add targeted single-file indexing, but it must preserve the same receipt schema.
- Smoke query: search for `{date + top topic + generated title}` and require the generated daily digest or reflection in top 5; otherwise mark `index_smoke_status=failed`.

Feedback-to-second-brain contract:

- Generated Jianling docs are indexed as `source_tier=generated_memory`, not `raw` and not `human_authored`.
- `generated_memory` is allowed to help recall and reranking, but ranking must preserve source-tier trace so users can see when an answer is based on generated synthesis versus raw evidence.
- Search results from generated docs must expose their underlying claim refs; `get_source` / `explain_result` must be able to jump from generated bullet -> source quote.
- Future Jianling runs may retrieve `generated_memory` as `prior_generated`, but promotion requires primary raw or human-authored evidence per §9.1.
- Release eval must include before/after cases proving generated docs improve recall for synthesized topics without demoting raw truth for direct source queries.

Source-tier ranking policy:

| Query intent | Default ordering / constraint |
|---|---|
| `direct_lookup` / 原文定位 / exact phrase / file path / date / quote match | `raw_truth` and `human_truth` have hard protection; `generated_memory` cannot outrank an exact raw/human hit. |
| `recent_session_recall` | primary raw/session evidence first; daily digest can appear as cited context. |
| `synthesis` / 总结 / 经验回顾 / lesson query | `generated_memory` may be boosted, but it must expose claim refs and source-tier badges. |
| `concept_query` | human-authored wiki/concepts first; generated concept proposals do not outrank active human-authored concepts by default. |

Generated result without resolvable claim refs is downgraded or hidden from default answer evidence.

---

## 13. CLI / API Surface

### 13.1 Setup

```bash
orderk jianling enable --schedule "03:30" --timezone Asia/Shanghai
orderk jianling disable
orderk jianling doctor --json
orderk jianling status --json
```

### 13.2 Running

```bash
orderk jianling run --date today
orderk jianling run --since 24h
orderk jianling run --weekly 2026-W24
orderk jianling run --monthly 2026-06
orderk jianling run --dry-run --json
```

### 13.3 Review / apply / rollback

```bash
orderk jianling list-proposals
orderk jianling show-proposal <id>
orderk jianling apply-proposal <id>
orderk jianling reject-proposal <id> --reason "too broad"
orderk jianling revert --run-id <run-id>
orderk jianling explain brain/reflections/foo.md
```

Proposal / apply / revert mechanics:

- Proposal format is Markdown with frontmatter plus embedded patch or new-file body; machine-readable copy is stored in receipt `proposals[]`.
- Every file mutation is represented as a `FileOp`: `create`, `patch`, `delete`, or `proposal_only`.
- `FileOp` must include target path, preimage hash, postimage hash, patch id, byte count, and whether index update is required.
- `apply-proposal` checks target preimage hash before writing; mismatch stops with conflict.
- `revert --run-id` is idempotent: created files are removed only if postimage still matches; patched files are restored only if current hash equals recorded postimage; otherwise revert stops and reports user-edit conflict.
- Revert never edits raw source files and never deletes manually edited files.

### 13.4 MCP tools

MCP should expose read-only Jianling status by default:

- `jianling_status`
- `jianling_doctor`
- `jianling_list_proposals`
- `jianling_get_run`

Write/apply/revert MCP tools should be disabled by default and require explicit local config allowlist, because proposal apply mutates Markdown. Remote MCP parameters must never self-authorize write tools.

Read-only MCP tool schemas must return compact JSON and include `profile`, `vault`, `scheduler_backend`, `last_run`, `next_run`, `status`, `warnings`, and `config_source` where relevant.

---

## 14. Observability / Audit / Receipts

Each run writes:

```text
.orderk/jianling/runs/2026-06-10T03-30-00.json
```

Receipt schema:

```json
{
  "schema_version": "orderk.jianling.run.v1",
  "run_id": "jianling-20260610-033000",
  "mode": "daily",
  "status": "success|degraded|failed",
  "success_predicate": {
    "provider": "called_success",
    "schema_validation": "passed",
    "pre_llm_guard": "passed",
    "pre_write_guard": "passed",
    "write": "passed",
    "index_smoke": "passed",
    "receipt_write": "passed"
  },
  "started_at": "...",
  "finished_at": "...",
  "vault": "/path/to/vault",
  "db": "/path/to/.orderk/index.sqlite",
  "profile": "default",
  "scheduler_backend": "systemd-user|launchd|windows-task|server-loop|manual",
  "llm_profile": "minimax-m3-standard",
  "embedding_profile_fingerprint": "...",
  "prompt_version": "jianling-daily-v1",
  "prompt_hash": "sha256:...",
  "template_id": "daily_digest.v1",
  "template_hash": "sha256:...",
  "schema_hash": "sha256:...",
  "template_registry_hash": "sha256:...",
  "evidence_pack_hash": "sha256:...",
  "evidence_pack_path": ".orderk/jianling/runs/<run-id>.evidence.json.redacted",
  "provider_status": "called_success|called_timeout|called_pii_guard_blocked|...",
  "schema_validation_status": "passed|failed",
  "budget_status": "within_budget|truncated|queued",
  "pre_llm_guard_status": "passed|blocked|redacted|local_only",
  "pre_write_guard_status": "passed|blocked|redacted",
  "index_update": "success|queued|failed",
  "index_smoke_status": "passed|failed|skipped",
  "fallback_used": false,
  "source_files": 18,
  "source_chars": 240000,
  "retrieved_context_chunks": 80,
  "llm_calls": 3,
  "generated_files": ["brain/daily/2026-06-10.md"],
  "generated_source_tier": "generated_memory",
  "importance": {
    "score_source": "rule|llm|hybrid",
    "score_confidence": 0.82,
    "breakdown": {
      "user_emotion": {"score": 2, "reason": "...", "evidence_ref": "..."},
      "failure_cost": {"score": 3, "reason": "...", "evidence_ref": "..."}
    }
  },
  "evidence_mix": {
    "primary_raw_quote_count": 3,
    "primary_evidence_char_ratio": 0.42,
    "prior_generated_context_ratio": 0.21,
    "generated_lineage_depth": 1
  },
  "dedupe": {
    "source_fingerprint": "sha256:...",
    "topic_fingerprint": "sha256:...",
    "action_rule_fingerprint": "sha256:...",
    "semantic_fingerprint": "sha256:...",
    "candidate_id": "sha256:...",
    "duplicate_decision": "new|link_existing|supersede_proposal|reject_duplicate|reject_generated_only_loop|skip_pre_llm_rejected_recent"
  },
  "proposals": [".orderk/jianling/proposals/...md"],
  "rejected_candidates": 12,
  "skipped_candidates_by_reason": {"chatter_noise": 9, "secret_guard": 1},
  "secret_findings": 0,
  "pii_findings": 0,
  "file_ops": [
    {
      "op": "create|patch|delete|proposal_only",
      "path": "brain/daily/2026-06-10.md",
      "preimage_hash": null,
      "postimage_hash": "sha256:...",
      "patch_id": null,
      "bytes": 12345,
      "index_required": true
    }
  ],
  "rollback_manifest": ".orderk/jianling/runs/2026-06-10T03-30-00.rollback.json",
  "watermark_advanced": true,
  "cost_estimate": {"input_tokens": 0, "output_tokens": 0},
  "warnings": []
}
```

No receipt may include secret values. A run can be `success` only when every `success_predicate` field is green. If generated Markdown is written but indexing fails, the run is `degraded`, not `success`.

---

## 15. Security / Privacy

1. **Secret guard before LLM**  
   Source bundles sent to cloud LLM are redacted. If a file is secret-heavy, skip and report.

2. **PII / private-fact guard before LLM**  
   PII taxonomy includes email, phone, address, government IDs, financial account details, health data, private relationship/family facts, and precise location traces. Policies: `block`, `redact`, `hash`, `local_only`, or `allow_by_config`. Default for cloud LLM is `redact` for ordinary contact info and `block/local_only` for high-risk identifiers.

3. **Secret and PII guard before write**  
   Generated Markdown is scanned. Any secret-like output blocks write or writes only to rejected quarantine with redacted content. PII output follows the same policy matrix and records `pii_findings` plus `pre_write_guard_status` in the receipt.

4. **Config never stores raw API keys**  
   Only `api_key_env` or OS keychain references.

5. **No raw mutation**  
   LLM cannot edit source transcripts.

6. **Profile guard**  
   Multi-vault / multi-profile installs must not cross-ingest or cross-write without explicit scope.

7. **Cloud disclosure awareness**  
   Enabling Jianling means configured source excerpts may be sent to the selected LLM provider. `orderk jianling enable` must say this clearly and require `--accept-cloud-llm` unless using a local model.

---

## 16. Resource Budgets

Target on a 2C/4G VPS:

| Component | Target |
|---|---:|
| Idle resident RSS | 0 for timer mode; <80MB for server embedded loop |
| Nightly worker RSS | <250MB p95 excluding OS cache |
| Daily wall time | <10 min p95 for 1 day of sessions |
| Weekly wall time | <20 min p95 |
| Daily generated docs | <= 1 daily + <=5 reflections/open-loops + <=8 proposals; facts default proposal-first |
| Disk growth from generated md | normally <5MB/day |
| LLM calls | <=8 daily, <=12 weekly |
| Query path LLM calls | 0 by default |

If budgets are exceeded, job must degrade/queue rather than run away.

---

## 17. Quality Gates / Acceptance Criteria

### P0 — Product contract gate

- V3 archive receipt exists and records GitHub Release `v0.1.17`, npm `orderk-cli@0.1.17`, asset digest, tarball URL, current V3 capability baseline, and the known CI caveat;
- V4 release gate must either fix/supersede the observed `v0.1.17` CI failure with a later CI-green release or mark V4 not release-ready;
- No external agent cron dependency;
- OrderK-managed schedule install/status/disable works and reports scheduler owner/backend;
- On Linux P0, installing OrderK + configuring LLM + `orderk jianling enable` can produce a scheduled next run without Hermes/agent/external scripts;
- Timer mode resident daemon count remains 0 except short-lived `orderk jianling run` worker; server-loop mode reports its resident process explicitly;
- Missing LLM config disables Jianling fail-closed;
- Raw transcripts are never mutated;
- Generated output is Markdown with valid frontmatter;
- Every `active_generated` or `active_user_approved` card has source refs and passes source-anchor resolution;
- Secret scan blocks unsafe output;
- Search/index still works without Jianling.

### P1 — Daily memory compiler gate

Fixture: 10 synthetic sessions with corrections, successes, failures, idle chatter, secrets, and repeated themes.

Pass:
- daily digest generated for meaningful days;
- generated files pass `orderk jianling validate-file` and the run passes `validate-run`;
- prompt/template/schema hashes are recorded in receipt;
- idle chatter not promoted;
- secret sample not written or sent unredacted;
- correction/failure generates reflection;
- reflection promotion records evidence mix, lineage depth, novelty/delta, recall cues, and evidence grade;
- weak “I reflected on memory” meta-chatter does not generate reflection;
- generated-only prior reflection does not generate a new reflection;
- fact-like claims become proposals by default; only direct-quote `user_stated|project_state|config_state` facts may use the narrow auto exception;
- output count within budget;
- receipt complete.

### P2 — Retrieval feedback gate

Fixture: before/after generated Markdown indexing.

Pass:
- after daily run, query for that day’s topic retrieves daily/reflection card in top 5;
- generated docs carry reranker evidence when searched through default path;
- generated docs are indexed as `source_tier=generated_memory` and expose claim refs back to raw/human-authored sources;
- runtime source-tier ranking policy is enforced: direct lookup/exact phrase/file/date/quote queries prefer raw/human hits, while generated memory may boost synthesis queries only with claim refs;
- same embedding profile is recorded;
- generated docs do not bury raw/source truth: for queries whose expected answer is a raw/source document, the raw/source hit must remain in top 3 unless the generated card is explicitly a cited synthesis of that raw source;
- golden query set has quantitative no-regression: Recall@10 and nDCG@10 may drop by at most 1 absolute query or 2%, whichever is stricter; MRR@10 may not drop by more than 0.02;
- any regression case is listed with before/after top 10, generated-doc involvement, and source-tier scores.

### P3 — Weekly/monthly consolidation gate

Fixture: 4 weeks of daily digests with repeated themes.

Pass:
- repeated reflections merge into lesson;
- lesson can propose skill/principle;
- duplicate cards are superseded or referenced, not copied forever;
- conflicts are surfaced as proposal, not silent overwrite.

### P4 — Real-vault dry-run gate

On a real vault copy:

- dry-run writes no files;
- reports predicted files, source refs, budgets, warnings;
- full run on copy generates bounded docs;
- `orderk jianling revert --run-id` removes generated files/proposals and restores modified files if any;
- `orderk doctor` remains green/degraded with explicit reasons.

### P5 — Longitudinal health gate

Run for 14 scheduled nights on a test vault.

Pass:
- no unbounded doc growth: generated files <= 120 over 14 days and generated Markdown <= 500KB unless test fixture explicitly exceeds normal workload;
- duplicate title/topic ratio <= 15%;
- death-loop fixtures pass: generated-only evidence and reflection-about-reflection are rejected, repeated weak meta-comments stay digest-only;
- no secret findings; PII findings follow configured policy and are recorded;
- failed provider days produce clear degraded receipts;
- weekly summaries reduce rather than increase noise: weekly output chars <= 40% of the 7 daily digest chars it summarizes, excluding source refs;
- search quality smoke does not regress under P2 thresholds;
- proposal/rejected retention pruning keeps `.orderk/jianling/proposals` and `rejected` within configured retention.

---

## 18. Implementation Phases

### Phase 0 — V3 archive, charter, schemas

- Create V3 archive receipt with GitHub/npm evidence before V4 boundary work;
- Freeze `.orderk/config.toml`, frontmatter, claim-source marker, source-anchor relocation, receipt, rollback manifest, proposal, lock, watermark, template registry, prompt registry, schema registry, reader/writer compatibility matrix, and registry rollback semantics;
- write fixture sessions and expected outputs for no-write/digest/reflection/fact-proposal/narrow-fact-exception/proposal/secret/PII/duplicate/death-loop/evidence-mix/ranking-policy classes;
- add doctor/status skeleton;
- define cloud LLM consent text;
- update only draft charter docs, not active README claims.

### Phase 1 — Linux OrderK-managed scheduler, no LLM writes yet

- Implement `jianling enable/disable/status/doctor`;
- Linux systemd user timer backend first, generated and owned by OrderK;
- explicit vault/DB/profile resolution;
- atomic lock, watermark, run receipt, stale-lock recovery;
- dry-run collects sources and reports; no LLM, no Markdown write.

### Phase 2 — LLM provider + daily compiler MVP

- Implement `LlmProvider::complete_structured` and MiniMax M3 profile;
- cloud consent and guard statuses;
- evidence retriever using existing OrderK search/rerank;
- structured LLM output validation;
- write one daily digest + limited validated reflections/open-loops; facts remain proposals except narrow direct-quote exceptions;
- reuse current whole-vault hash incremental index and run smoke query.

### Phase 3 — Governance, proposals, rollback

- proposal writer/apply/reject/revert with FileOp manifest;
- secret + PII guard before LLM and before write;
- conflict/supersede handling;
- claim-level source refs and `explain`.

### Phase 4 — Weekly/monthly consolidation

- weekly merge;
- monthly promotion candidates;
- duplicate/stale management;
- stable-layer changes remain proposal-first.

### Phase 5 — Cross-platform scheduler + MCP/read-only observability

- macOS LaunchAgent and Windows Task Scheduler backends with platform tests;
- server-loop scheduler only if `orderk serve` exists;
- read-only MCP status/proposal listing;
- optional TUI/UI proposal review later;
- no write MCP by default.

### Phase 6 — Annual review proposal

- annual review digest/proposal only;
- no automatic constitution rewrite;
- stable principles/profile changes remain user-approved proposals.

---

## 19. Risks and Countermeasures

| Risk | Countermeasure |
|---|---|
| Becomes Hindsight-heavy | One nightly loop; Markdown truth; no Postgres/control-plane; strict budgets |
| Generates noise | importance threshold; daily caps; weekly/monthly dedupe; death-loop fixtures |
| Reflection loops on itself | generated-only evidence cannot promote; reflection-about-reflection rejected; generated docs marked `source_tier=generated_memory` |
| Template/prompt drift breaks auditability | template/prompt/schema registries; receipt hashes; validate-template/validate-run gates |
| Corrupts stable knowledge | candidate/active separation; proposal-first for stable layers |
| Leaks secrets to LLM | pre-LLM redaction; skip secret-heavy files; no raw API keys in config |
| Provider failure creates silent gaps | explicit degraded receipts; fail-closed semantics |
| Search path slows down | query-time LLM = 0 by default; Jianling runs off-path |
| Cross-profile pollution | profile/vault scope guard and locks |
| Generated docs become trusted too early | `status=draft/proposed/active_generated/active_user_approved`, source refs, confidence, supersedes |
| Index drift | generated docs reindexed with profile fingerprint; doctor detects stale profile |
| External cron dependency sneaks back | setup/status/disable owned by OrderK; Hermes/agent cron explicitly non-goal |

---

## 20. Product Verdict

Jianling should be OrderK’s first truly active capability, but it must be active in the **sleep cycle**, not in the query path.

The full product shape is:

```text
OrderK = fast evidence search blade
Jianling = built-in nightly Markdown memory compiler
Obsidian/vault = source of truth
LLM = bounded reflection engine
Index = rebuildable cache
```

If built this way, OrderK can keep its current advantage — lighter than systems with equal quality, higher quality than systems equally light — while gaining the one missing organ: **sleep-after reflection that turns repeated experience into durable Markdown knowledge.**
