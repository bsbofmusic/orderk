# OrderK V4 Jianling / Sword Spirit PRD — Built-in Sleep Reflection & Markdown Memory Compiler

> Status: **V4 production line: `0.1.21` launched the P0/P1 scheduler/compiler slice; the current 2026-06-15 v41 line on `0.1.28` adds K-voice historian reflection, default-on live LLM activation when a valid chain/key pointer exists, bounded post-write index feedback, receipts/validators, OrderK-managed `systemd --user` timer, and writer/auditor/foreman gates. Full autonomous approval UX remains gated.**
> Date: 2026-06-10
> Owner intent: 茶老板提出“剑灵是睡后反思者”，不是外部 Hermes/agent cron，而是 OrderK 自身安装后、配置 LLM 后无感运转的内嵌夜间记忆整理机制。
> Naming note: The new Jianling / Sword Spirit product generation is **OrderK V4**. V4 is built after the current V3 baseline, but it is not a patch label for V3. The old V3 release line must be archived clearly in npm, GitHub Releases, CHANGELOG, and repo docs before V4 changes the product boundary. Old `orderk-v2-sword-spirit-prd.md` remains historical and is not the active V4 design source.

---

## 1. 一句话定义

**OrderK Jianling 是 OrderK 内置的“睡后反思者”：它在夜间读取当天新增的原始 session / Markdown 事件，用 OrderK 自己的源选择和后续检索上下文，把当天经验沉淀成可读、可审计、可 Git 管理的 Markdown。P0/P1 的正式成功条件是 Markdown、receipt、evidence、log、scheduler、以及至少一个 bounded `orderk index --path <generated.md>` + retrieval smoke 全部可验；全库自动 reindex 和更强的 autonomous proposal UX 仍是后续 gated 阶段。**

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
- Workspace version files for the P0/P1 implementation line say `0.1.19`.
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

This PRD records the V4 product-boundary change. The `0.1.21` release promoted the P0/P1 Jianling scheduler/compiler slice. The current `0.1.28` / v41 production line keeps `orderk jianling` as an explicit Markdown compiler sidecar with an OrderK-managed active `systemd --user` timer, `jianling worker --once` planner, receipts/evidence packs, validators, safety gates, profile-wide locks, persistent run logs, self-check/chat-smoke, live Anthropic-compatible MiniMax M3 reflection, and verified single-file `orderk index --path` feedback for generated Markdown. The search/MCP query path remains read-only. Live reflection activation resolves in this order: per-profile override, global override, then default-on when a valid LLM chain/key-env pointer exists.

Promotion rule:

1. Old `orderk-v2-sword-spirit-prd.md` remains historical and must not be used as the active design source.
2. `0.1.21` user-facing docs may say “OrderK has Jianling V4 P0/P1 production scheduler”; current docs should say `0.1.28` / v41 has historian reflection and default-on live LLM activation when a valid chain/key pointer exists.
3. User-facing docs must not claim unmanaged autonomous writes: scheduler activation, write targets, receipts, guards, and source anchors stay explicit; `chat-smoke` is the separate live connectivity command.
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
- P0/P1 live reflection target uses the existing Sword model profile plumbing: default LLM profile is Anthropic-compatible `MiniMax-M3`, credentials are read through `ORDERK_SWORD_LLM_API_KEY_ENV` or OrderK-scoped LLM key envs, and raw API key values are never written to config, receipts, PRD, or logs.

### 2.4 2026-06-11 implementation update / learned gates

This update records the hard lessons from the V4 implementation drill. It is part of the PRD contract, not a chat-only after-action note.

| Question | PRD answer after implementation |
|---|---|
| Is Jianling an explicit switch? | Scheduler ownership is explicit: `orderk jianling enable/disable` controls the systemd timer. Historical `0.1.21` live reflection used an explicit LLM hot switch. Current v41 resolves live LLM activation as per-profile override, then global override, then default-on when a valid LLM chain/key-env pointer exists. Setting either switch false still disables the live call intentionally. |
| Is it hot-swappable? | Yes for the live reflection slot: model/profile/key env/base URL are resolved at run time through the existing Sword model profile. Changing env/profile then rerunning `chat-smoke`, `self-check`, or `run` is enough; no rebuild is required. |
| Do daily/weekly/monthly runs conflict? | They must not. The implemented lock is profile-wide (`.orderk/jianling/locks/<profile>.lock`), not per-mode; daily, weekly, and monthly runs for the same vault/profile are mutually exclusive. Receipts record mode/run-id and the lock path. |
| What if the evidence exceeds the LLM/context budget? | The run must fail closed or become explicit partial. Current P0/P1 uses a Kanban harness: writer cards draft bounded evidence slices, auditor cards check format/traceability against writer drafts and final Markdown draft, and a foreman manifest gates the final Markdown write. Receipts record `partial_source_file_limit`, rejected source paths, chunk count, chunk dir, and foreman manifest path. Silent truncation is forbidden. |
| Is the current LLM MiniMax M3? | The default live slot is `anthropic:MiniMax-M3:<profile_fingerprint>`. Live smoke and live run receipts must show model/profile fingerprint without exposing API key values. |
| Does generated reflection keep both ledger and observation? | Yes. From `0.1.23`, generated Markdown uses `digest_schema_version: orderk.jianling.digest.v2` and `reflection_layers: [factual_ledger, reflective_synthesis]`. The digest must keep a seven-section shape: `一句话结论`, `客观底账 / Factual ledger`, `推断观察 / Reflective synthesis`, `用户/系统模式 / User-system patterns`, `未闭合风险 / Open risks`, `下次动作 / Next actions`, and `证据附录 / Evidence appendix`. Every observation should carry evidence refs, confidence, and a next action. This is the V3→V4 boundary: V3 retrieves/compresses facts; V4/Jianling refines facts into memory with judgment, similar to Hindsight/Hermes compaction. |
| Has a 2026-06-01..2026-06-10 drill run? | Yes, local acceptance evidence ran 10 daily runs plus one weekly run on 2026-06-07 and one monthly run on 2026-06-10. All 12 receipts used `provider_status=called_live`, passed `validate-run`, wrote weekly/monthly to PRD paths, and exercised Kanban writer/auditor/foreman + explicit partial behavior. |

Implementation evidence captured in this repo update and the 2026-06-11 production launch:

- `/home/agent/.local/bin/orderk --version` and repo `target/release/orderk --version` both report `0.1.21`; stale in-memory MCP binaries must be restarted after deploy.
- Live `orderk jianling chat-smoke` with `ORDERK_SWORD_LLM_API_KEY_ENV` pointing at the MiniMax key env returned `ok=true`, `status=connected`, `provider=anthropic`, `model=MiniMax-M3` and wrote a smoke receipt.
- `orderk jianling enable --vault /home/agent/obsidian-vault --profile default --schedule 03:30 --timezone Asia/Shanghai --db /home/agent/obsidian-vault/.obsidian/orderk/orderk-clean.sqlite --orderk-bin /home/agent/.local/bin/orderk` wrote `/home/agent/.config/systemd/user/orderk-jianling@default.{service,timer}`, `/home/agent/.config/orderk/default.env`, and `.orderk/jianling/scheduler.json`.
- Live `systemd --user` state: `orderk-jianling@default.timer` is `enabled`, `active (waiting)`, next run `Fri 2026-06-12 03:34:04 CST`; `systemd-analyze --user verify` exits 0.
- Manual production entrypoint verification used `systemctl --user start orderk-jianling@default.service`; it ran `/home/agent/.local/bin/orderk jianling worker --once ...`, produced scheduled run `jianling-20260611T115446Z-2588643`, `provider_status=called_live`, generated `brain/daily/2026-06-11.md`, and wrote receipt/evidence/log under `.orderk/jianling/`.
- `orderk jianling doctor` is `ok=true`; `orderk jianling validate-run --run-id jianling-20260611T115446Z-2588643` is `ok=true`; `orderk jianling chat-smoke` is `ok=true`, `status=connected`, `model=MiniMax-M3`.
- Quality gates passed after the launch fix: `cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, focused `cargo test -p orderk-core --test jianling_contract` (19 tests), profile slot regression tests (8 tests), incremental `index --path` regression, and `cargo build --release --locked`.
- The generated daily Markdown was indexed without full rebuild using `orderk index --path brain/daily/2026-06-11.md`; the summary was `ok=true`, `files=1`, `added=1`, `deleted=0`, `chunks=7`, `embedded=7`, embedding profile `siliconflow:Qwen/Qwen3-Embedding-4B`. `orderk search --query jianling-20260611T115446Z-2588643 --view index --reranker none` returned `brain/daily/2026-06-11.md` in top results, proving the generated run is searchable through the active OrderK DB.
- The 2026-06-01..2026-06-10 gated drill returned `ok=true`, `runs=12`, `daily_runs=10`, `weekly_runs=1`, `monthly_runs=1`, `multi_chunk_runs=6`, `partial_runs=3`, `total_writer_cards=18`, `total_auditor_cards=18`; weekly output path was `brain/weekly/2026-06-07.md`, monthly output path was `brain/monthly/2026-06-10.md`. Every non-empty run emitted `writer-*.json`, `auditor-*.json`, and `foreman-manifest.json`; final Markdown was written only after foreman acceptance.

Open boundaries that are **not** silently claimed by this update:

- The implemented live reflection writes a bounded LLM reflection section into generated Markdown; full structured card extraction, proposal apply/revert UX, and claim-level `explain` remain later gates.
- Query-time search remains LLM=0 by default.
- Whole-vault automatic reindex policy remains a later release/eval gate. P0/P1 now wires the safer bounded path into each non-dry Jianling run: after generated Markdown is written, the worker runs single-file `orderk index --path <generated.md>` through the active DB/profile, performs a retrieval smoke without external reranker, records `index_summary`, and marks the run degraded if the index or smoke gate fails.

### 2.5 Reflective action loop / documentation routing / config-log contract

The 0.1.21 production drill changed the operating model: Jianling is not a one-time repair script. It is OrderK's reflective operating loop. A hard run is only complete when the experience has moved through this loop:

```text
live state -> diagnose -> fix/gate -> real smoke -> bounded index/search feedback -> distill lesson -> route docs -> verify docs
```

Landing-surface routing is part of the product contract:

| Lesson class | Durable landing surface |
|---|---|
| Product boundary, acceptance gates, roadmap semantics | this PRD / `docs/charters/README.md` |
| Repeatable operator procedure, pitfalls, full-open audits | `orderk-jianling-production-ops` skill |
| Long run evidence, exact commands, run IDs, historical receipts | skill `references/*.md` and Obsidian system records |
| User-facing second-brain truth | Obsidian `brain/systems/*.md` |
| Search/MCP read-only route rules | `orderk-search-blade` skill |
| Mechanical recurrence prevention | Rust tests, release gate, doctor/status/chat-smoke, or a smoke probe |

Configuration and logging are layered, not a single hidden daemon:

| Surface | Current role |
|---|---|
| CLI flags | Highest-confidence runtime input: vault, DB, profile, provider/model, `--path` |
| SQLite `settings` | Active index profile and vector backend truth; verify via `orderk status` |
| Env model slots | Embedding/reranker/LLM slot resolution; key env names are pointers, not secrets |
| `/home/agent/.config/orderk/<profile>.env` | systemd-user scheduler profile env with flags/pointers only |
| `.orderk/jianling/scheduler.json` | scheduler ownership/state inside the vault |
| `.orderk/jianling/runs/*.json` | machine receipts for every Jianling run |
| `.orderk/jianling/runs/*.evidence.json.redacted` | redacted source/evidence packs |
| `.orderk/jianling/logs/*.log` | compact human-readable run logs |
| `.orderk/jianling/smoke/*.json` | live LLM smoke receipts |
| journald user unit | service runtime evidence for `orderk-jianling@<profile>.service` |
| MCP tool output | live agent-facing search surface; must be tested after binary replacement |

A run must not be called “满血全开” merely because config files look right. It must verify active binary, active DB, default reranker, fresh MCP process, MCP tool calls, Jianling scheduler, live LLM smoke, latest receipt/log, and bounded index/search feedback. Stale `/home/agent/.local/bin/orderk (deleted)` MCP processes are an explicit P1 operational hazard after deploy.

### 2.6 Production status update — 2026-06-15

As of the 2026-06-15 production verification, the active V4 line is no longer only the original `0.1.21` P0/P1 scheduler slice. The deployed production binary at `/home/agent/.local/bin/orderk` is built from the v41 tree on the `0.1.28` version line (the version string was not bumped by the v41 patch set, so production verification must use binary fingerprints, not `orderk --version`). The decisive v41 fingerprints are UTF-8 prompt markers such as `夜班日记`, `今日主线`, `我的看法`, and `K 的夜班` compiled through `include_str!("assets/jianling_historian_prompt.md")`.

The current accepted V4 behavior is:

1. **Search/MCP remains read-only.** `orderk search/get/status/health/doctor/mcp` do not mutate the vault.
2. **Jianling is the explicit write-capable compiler.** It writes generated Markdown under `brain/daily|weekly|monthly|yearly|lessons` only through receipts, guards, source anchors, and the writer/auditor/foreman gate.
3. **Historian reflection is the main human-facing layer.** The LLM writes a K-voice night-shift diary; deterministic anchors, hashes, and counts are evidence material, not the whole reflection.
4. **Live reflection is production-verified through MiniMax M3.** A connected `chat-smoke` result is `ok=true`, `status=connected`, and `response_preview=orderk-jianling-smoke-ok`; false negatives can occur if a manual shell omits the systemd user-manager environment containing the key-env pointer.
5. **Operational evidence is part of DR.** `.orderk/` contains Jianling run receipts, redacted evidence packs, logs, smoke receipts, scheduler/watermark/audit state, and eval cases. It is now a declared Obsidian DR backup surface after `dr_audit` rejected it as unclassified during the 2026-06-15 backup repair.

Open caveats remain explicit: natural timer proof requires the next `systemd --user` trigger receipt, and a generated Markdown file is only considered searchable when the post-write index feedback and active DB file hash/size freshness checks pass.


---

## 3. Goals / Non-goals

### 3.1 Goals

1. **内置调度，不依赖外部 agent cron**
   用户安装 OrderK 并配置 LLM 后，OrderK 自己管理 nightly reflection schedule。可以通过系统 timer/service 实现，但 timer 的创建、状态、日志、禁用、恢复均由 `orderk` 命令管理，不依赖 Hermes cron、外部 agent、手写 shell glue。

2. **Markdown-first consolidation**
   Jianling 的所有长期产物必须是 `.md`；SQLite/vector/reranker cache 只是索引，可删可重建。

3. **睡后反思，不阻塞白天搜索**
   默认 search/query path 不调用 LLM 反思；Jianling 在夜间或用户手动 `orderk jianling run` 时运行。

4. **正反馈闭环（P0/P1 bounded path, P2 full-vault/planner gate）**
   `raw session -> Jianling digest/reflection md -> automatic single-file orderk index feedback -> retrieval smoke -> future search/rerank better -> next Jianling has better context`。P0/P1 要求每个 non-dry generated Markdown run 自动执行 bounded `index --path` 并通过 retrieval smoke；全库策略和更强的 context-feedback planner 才是后续门禁。

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
orderk jianling worker --once --profile <profile> --scheduled-equivalent
```

`worker --once` is the OrderK-owned one-shot scheduler worker. It plans daily every run, weekly on Sunday, monthly on day 1, and yearly on Jan 1, then serializes those modes through the same profile-wide lock and `jianling run` receipt path.

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
ExecStart=<absolute-orderk-bin> jianling worker --once --profile <profile> --vault <absolute-vault> --db <absolute-db>
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
- `doctor` / `self-check` validates binary path exists, env file exists, LLM key env name exists without printing value, vault/DB paths exist, profile-wide lock availability, output path policy, latest receipt freshness, and scheduler ownership. `chat-smoke` is the live LLM probe and writes its own smoke receipt.
- Missed runs use `Persistent=true`; after wake-up, only one catch-up run is allowed per profile.
- DST/timezone ambiguity is resolved by configured timezone; if unsupported by backend, receipt records `timezone_backend=system_local`.
- Logs go to journald plus `.orderk/jianling/logs/<run-id>.log` with secret redaction.

### 6.4 Installation flow

```bash
orderk init --vault ~/obsidian-vault
# config/profile path, or env-only for the current implementation slice:
export ORDERK_SWORD_LLM_API_KEY_ENV=MINIMAX_API_KEY
# Optional override. If absent, v41 defaults live reflection on when a valid LLM chain/key pointer exists.
# export ORDERK_JIANLING_LLM_ENABLED_DEFAULT=1
orderk jianling chat-smoke --vault ~/obsidian-vault --profile default
orderk jianling self-check --vault ~/obsidian-vault --profile default
orderk jianling enable --schedule "03:30" --timezone Asia/Shanghai
orderk jianling doctor --vault ~/obsidian-vault --profile default
```

If LLM credentials are not configured, `chat-smoke` reports `llm_unconfigured` and writes a failed smoke receipt; Search/index still works. If credentials are configured and no enable/disable override is set, v41 treats the valid chain as live-enabled by default. Explicit false/off overrides remain the emergency kill switch.

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
    "min_orderk_version": "0.1.19",
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

#### 8.1.1 Cross-day background context (V4)

Daily/manual runs are no longer amnesiac. In addition to today's raw transcripts
(the `[S#]` evidence tier), the run loads the previous `JIANLING_DAILY_BACKGROUND_DAYS`
(default 7) of **generated** `brain/daily/*.md` reflections as a read-only `[BG#]`
background tier. Rules enforced in code (`crates/orderk-core/src/jianling.rs`):

- **Window is `[date-7, date-1]`, end-exclusive** (`source_file_in_background_window`):
  a daily run never ingests its own `brain/daily/<date>.md` output.
- Only **managed** files (`generated_by: orderk-jianling`) qualify as background.
- The LLM evidence pack renders two labelled sections — `BACKGROUND (read-only…)`
  and `TODAY'S EVIDENCE` — and both the generate and repair prompts carry an explicit
  anti-copy clause.
- The contract validator (`validate_live_llm_reflection_contract`) requires every
  Observations section to cite **at least one `[S#]` today-evidence anchor**; a
  `[BG#]`-only reflection is rejected (then repaired). `[BG#]` can never satisfy the
  citation contract, because the validator is only ever passed today's `S`-anchors.
- `max_tokens` is the named constant `JIANLING_LLM_MAX_TOKENS` (2000).
- The run receipt surfaces `source_background_files` and `llm_max_tokens` for transparency,
  and the evidence pack carries `background_generated_sources` plus a `primary_raw_sources`
  subset filtered to the `raw_truth` tier (accurate for weekly+ rollups too).

**Residual risk (D):** anti-copy is enforced at the prompt level only; the validator does
not detect paraphrase or near-verbatim restatement of a background bullet. The `[S#]`
citation gate bounds this (a laundered conclusion still needs a real today-evidence anchor),
but a follow-up n-gram / embedding-similarity check against `background_sources` is the
recommended hardening. A second residual: prior generated reflections are now sent to the
LLM as prompt context, so anything written into a managed daily file becomes prompt input
(excerpts are still passed through `redacted_excerpt`).


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
- `brain/weekly/YYYY-MM-DD.md` for the weekly closing date;
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
- `brain/monthly/YYYY-MM-DD.md` for the monthly snapshot date;
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

Lock key is `{vault_id, profile}` for the active P0/P1 implementation. The lock is intentionally profile-wide, so `daily`, `weekly`, `monthly`, `yearly`, and manual runs for the same vault/profile cannot overlap. Lock file lives under `.orderk/jianling/locks/<profile>.lock` and uses atomic create-new semantics with `{pid, host, started_at, ttl_seconds, binary_path, run_id, mode}`. Stale lock recovery requires either TTL expiry or explicit `orderk jianling unlock --run-id`.

Watermark state lives in `.orderk/jianling/watermarks.json` and records source path, content hash, mtime, last_processed_run, and last_status. Generated Markdown under `brain/` / `wiki/` must be excluded from raw-source collection unless the phase explicitly reads prior generated digests.

Transaction order:

```text
global_profile_lock -> collect -> plan -> pre_llm_guard -> retrieve -> optional llm when explicit switch is enabled -> validate -> pre_write_guard -> write/propose -> index smoke/queue -> receipt -> advance watermark
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
- index update succeeds synchronously for the generated file;
- at least one smoke query can retrieve the generated digest/reflection by run_id + title;
- doctor/status surfaces expose degraded last-run state instead of hiding index failures.

If indexing fails, generated Markdown remains a generated artifact on disk, but it is not considered searchable synthesis until reindex succeeds; run status is `degraded_index_failed`. Raw/human-authored evidence remains the truth layer.

MVP index integration contract:

- P0/P1 implements the safer bounded path `orderk index --path <vault-relative.md>` for generated Markdown and wires that path into non-dry Jianling runs; full-vault hash-based indexing remains available for maintenance/rebuilds but is not required for Jianling launch smoke.
- Scheduled runs must pass explicit `--vault` and `--db` from config; cwd must not decide the active vault.
- Receipt records the bounded index method through `index_update`, `index_smoke_status`, and `index_summary` (`files`, `added`, `updated`, `chunks`, `embedded`, profile/backend, chunk options, and duration).
- If indexing or retrieval smoke fails, generated Markdown remains on disk but the run is marked `degraded_index_failed` with warnings; it is not silently reported as full success.
- Smoke query: search for `{run_id + generated title}` without external reranker and require the generated daily digest or reflection in top 5; otherwise mark `index_smoke_status=failed`.

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
orderk jianling self-check --json
orderk jianling chat-smoke --json
orderk jianling status --json
```

### 13.2 Running

```bash
orderk jianling run --mode daily --date 2026-06-10 --json
orderk jianling run --mode weekly --date 2026-06-07 --json
orderk jianling run --mode monthly --date 2026-06-10 --json
orderk jianling run --dry-run --mode daily --date 2026-06-10 --json
# Optional override: v41 defaults live reflection on when a valid LLM chain/key pointer exists.
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
  "provider_status": "called_live|configured_inactive_explicit_switch_off|configured_not_called_dry_run|llm_unconfigured_skipped|called_timeout|called_pii_guard_blocked|...",
  "schema_validation_status": "passed|failed",
  "budget_status": "within_budget|partial_source_file_limit|queued|failed",
  "pre_llm_guard_status": "passed|blocked|redacted|local_only",
  "pre_write_guard_status": "passed|blocked|redacted",
  "index_update": "success|failed|pending|skipped_no_db|skipped_dry_run",
  "index_smoke_status": "passed|failed|pending|skipped_no_db|skipped_dry_run|skipped_index_profile_failed|skipped_index_provider_failed|skipped_index_failed",
  "index_summary": {
    "path": "brain/daily/2026-06-10.md",
    "files": 1,
    "added": 1,
    "updated": 0,
    "unchanged": 0,
    "deleted": 0,
    "chunks": 7,
    "embedded": 7,
    "reused": 0,
    "embedding_provider": "siliconflow|mock|openai|...",
    "embedding_model": "Qwen/Qwen3-Embedding-4B",
    "vector_backend": "sqlite_vec|exact",
    "chunk_strategy": "heading|heading_overlap",
    "chunk_max_chars": 1200,
    "chunk_overlap_chars": 0,
    "took_ms": 1234
  },
  "fallback_used": false,
  "source_files": 18,
  "source_chars": 240000,
  "source_total_files": 60,
  "rejected_source_files": ["raw/transcripts/.../skipped.md"],
  "chunking_status": "not_needed|planned_kanban_chunks|kanban_foreman_summary_written",
  "chunk_count": 3,
  "chunk_dir": ".orderk/jianling/runs/<run-id>.chunks",
  "foreman_summary_path": ".orderk/jianling/runs/<run-id>.chunks/foreman-manifest.json",
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
      "op": "create|replace|delete|proposal_only",
      "target_path": "brain/daily/2026-06-10.md",
      "preimage_hash": null,
      "postimage_hash": "sha256:...",
      "byte_count": 12345,
      "index_update_required": true
    }
  ],
  "rollback_manifest": ".orderk/jianling/runs/2026-06-10T03-30-00.rollback.json",
  "watermark_advanced": true,
  "cost_estimate": {"input_tokens": 0, "output_tokens": 0},
  "warnings": []
}
```

No receipt may include secret values. A run can be `success` only when every `success_predicate` field is green. If generated Markdown is written but indexing fails, the run is `degraded`, not `success`. Every non-empty run must emit a Kanban harness under `<run-id>.chunks/`: `writer-*.json` drafts bounded source slices, `auditor-*.json` checks format standard and traceability by reading writer drafts, and `foreman-manifest.json` gates the final Markdown write. If the source set exceeds the configured file/context budget, receipt status must be explicit (`partial_source_file_limit` plus rejected paths and Kanban metadata); silent truncation is a release blocker.

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
- Missing LLM config fails closed for live reflection and is visible in `chat-smoke` / `self-check` receipts; current v41 live activation resolves as per-profile override, then global override, then default-on when a valid chain/key-env pointer exists; explicit false/off overrides intentionally disable the live call;
- Raw transcripts are never mutated;
- Generated output is Markdown with valid frontmatter;
- Every `active_generated` or `active_user_approved` card has source refs and passes source-anchor resolution;
- Secret scan blocks unsafe output;
- Search/index still works without Jianling;
- `orderk jianling self-check` reports LLM profile, profile-wide lock, brain output paths, scheduler/last-run status, and does not require the user to discover broken state manually;
- `orderk jianling chat-smoke` performs a live MiniMax M3 connectivity check and writes a redacted receipt without leaking API key values.

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
- receipt complete;
- live reflection path is covered by at least one fake-provider contract test and one real MiniMax M3 smoke/run drill before release evidence is claimed.

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
- conflicts are surfaced as proposal, not silent overwrite;
- weekly and monthly runs use PRD paths `brain/weekly/YYYY-MM-DD.md` and `brain/monthly/YYYY-MM-DD.md`, never the old `brain/reflections/weekly-*` / `monthly-*` bucket;
- profile-wide lock prevents daily/weekly/monthly overlap for the same vault/profile.

### P3.1 — 2026-06-01..2026-06-10 live drill gate

Fixture: a real or synthetic vault with daily source files from 2026-06-01 through 2026-06-10, `ORDERK_SWORD_LLM_API_KEY_ENV` pointing to the configured MiniMax M3 key env, and no false/off LLM override; include one subcase with `ORDERK_JIANLING_LLM_ENABLED_<PROFILE>` unset to prove v41 default-on behavior.

Pass:
- 10 daily runs, one weekly run at 2026-06-07, and one monthly run at 2026-06-10 complete with `provider_status=called_live`;
- every run passes `orderk jianling validate-run`;
- every non-empty run passes Kanban writer/auditor/foreman before final Markdown write; writer cards contain `draft_markdown`/`draft_hash`, auditor cards reopen writer drafts and verify `format_standard`/`traceability`/`draft_hash`, and large evidence sets produce multiple writer/auditor cards with explicit partial status when the file limit is exceeded;
- no old weekly/monthly reflection path is created;
- final report records run counts, chunked runs, partial runs, generated paths, and receipt locations.

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

### Phase 1 — Linux OrderK-managed scheduler, receipts, validators, and self-check

- Implement `jianling enable/disable/status/doctor/self-check/chat-smoke`;
- Linux systemd user timer backend first, generated and owned by OrderK;
- explicit vault/DB/profile resolution;
- profile-wide atomic lock, watermark, run receipt, stale-lock recovery;
- dry-run collects sources and reports; deterministic run writes guarded generated Markdown only, never raw transcripts;
- self-check must make broken LLM/lock/path/timer/receipt state visible without waiting for a user complaint.

### Phase 2 — LLM provider activation + daily compiler MVP

- Implement Anthropic-compatible MiniMax M3 live slot using Sword model profile plumbing and `ORDERK_SWORD_LLM_API_KEY_ENV` indirection;
- `chat-smoke` performs the live LLM probe and writes redacted smoke receipt;
- `jianling run` calls live LLM when v41 activation resolves true: per-profile override, global override, or default-on through a valid LLM chain/key-env pointer; false/off overrides record intentional inactive status;
- cloud consent and guard statuses;
- bounded evidence bundle with fail-closed or explicit partial/chunk/foreman metadata;
- write one daily digest with optional live reflection section and automatic single-file index feedback + retrieval smoke; full structured card extraction, proposal apply/revert UX, full-vault strategy, and claim-level explain remain later gates;

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
| Provider failure creates silent gaps | `chat-smoke`/`self-check`, explicit inactive switch state, degraded/failed receipts; fail-closed semantics |
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
