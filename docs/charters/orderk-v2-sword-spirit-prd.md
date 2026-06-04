# orderk V2 构想 / PRD：有剑灵的 Markdown-first 智能检索宝具

> Draft created: 2026-06-04T01:20:13+08:00  
> Status: revised PRD after GPT / SF DeepSeek V4 Pro / MiMo / MiniMax 四路审计  
> Owner intent: 茶老板希望 orderk V2 从被动检索刀，升级为快准狠稳、能主动唤醒推理、带单一后台剑灵机制的智能级检索工具。

## 1. 一句话定义

**orderk V2 是一个 Markdown-first 的智能检索宝具：以 Rust 做刀身、云端 LLM/embedding/reranker 做剑灵，在后台用唯一的 sword-spirit digest 把新增 raw 材料接入旧知识网络，让检索从“被动找得到”进化到“主动唤醒推理、越用越准”。**

更短的产品 slogan：

**从被动检索刀，到有剑灵的知识宝具。**

## 2. 与 Hindsight 的区别

Hindsight 是完整主脑系统：来料、拆解、实体、图谱、recall、reflect、consolidation、mental model、worker、operation queue 都很完整，优点是准、稳、会长期形成理解，坏处是重、慢、维护成本高。

orderk V2 不复制这艘航母。orderk V2 只保留一个剑灵 digest 点：**raw Markdown 进入后，后台 digest job 用 embedding 找旧知识邻居，用 LLM 提出概念/关系更新，用 reranker 去噪和加权，把存量 x 变成更厚的 x。** 查询时默认不 reflect，不现场开大会，而是站在已经被剑灵消化过的 wiki/graph 上快速检索、必要时再主动唤醒推理。

费曼比喻：V1 像普通刀，只负责切得快；V2 像带剑灵的刀，平时不多话，只有新材料进来后才在后台对照旧剑谱，把可靠关系写成可审计 proposal。

## 3. 背景判断：V1 / 旧 V2 草案的问题

orderk V1 的价值是快、轻、可验证，像一把检索刀。它能扫 Markdown、切块、嵌入、索引、搜索、返回证据。但它的问题也明显：它太线性，缺少反馈、消化、反哺。新资料 y 进来后，系统只是多了一批能被搜到的 chunk，旧知识 x 没有真正被更新。仓库里多了一批材料，但旧知识网络没有变厚。

早期 V2 草案强化了 Markdown-first、Rust、Tauri、cloud model、证据追踪，这些方向仍然对。但早期草案太强调“战机刀锋”，没有吸收 Hindsight 最有灵魂的地方：观察和消化。没有剑灵，V2 只是更漂亮、更快、更可控的搜索器；有了单一剑灵，它才可能成为轻量但会成长的知识系统。

因此，V2 的核心升级不是“加 UI”，也不是“加更多模型”，而是**让知识库本身越用越厚**：每次 raw 进入，不只是入库，还要和旧 wiki/graph 发生一次受控的化学反应。

## 4. 核心设计原则

第一，**Markdown 是真相源**。raw、source、concept、claim、decision、digest patch 都应尽量落成 Markdown 或可读 JSONL；SQLite/Tantivy/vector index 都只是可重建的库存本。数据库坏了，系统能从 Markdown 重建；模型换了，embedding 能重算；图谱乱了，能从 links/audit 重建。

第二，**只有一个剑灵**。Hindsight 到处 reflect，orderk V2 不能这样。V2 的剑灵只放在后台 digest：raw 入库后，增量消化新材料和旧知识的关系。普通搜索不 reflect，普通写入不 reflect，普通 chunk 不 reflect。这样保快和狠。

第三，**query-time reflect 降级为“主动唤醒推理”**。当查询命中稀疏、存在冲突、用户要求架构判断/复盘/统综、或检索结果低置信时，才进入 reasoning mode。它不是默认答案生成器，而是异常/高价值场景的主动唤醒。

第四，**成长必须可审计**。LLM 不直接成为库老板。LLM 只能提出候选概念、候选关系、候选更新；reranker 和规则门禁筛噪；高置信边可自动落库，中置信进入待审队列，低置信丢弃。重要概念页修改默认生成 patch，可人工确认。

第五，**快准狠稳要同时有数字和边界**。快：本地检索默认不等 LLM；准：BM25/vector/RRF/reranker/graph expansion 有 score trace；狠：砍掉多租户、云同步、重 worker、大 Control Plane、默认 chat；稳：raw 可读、index 可重建、profile guard、secret guard、doctor/eval/golden queries。

## 5. 产品定位

orderk V2 是智能检索工具，不是完整第二大脑主脑，不是聊天机器人，不是笔记 App 全家桶，也不是 Hindsight 平替。它的主任务是：**从 Markdown 知识底座中快速找到证据，并通过单一后台剑灵机制持续增强 wiki/graph，让未来检索更准、更会主动唤醒推理。**

如果说 V1 是“被动检索刀”，V2 是“有剑灵的宝具”。剑灵不是到处讲话，而是在后台修炼剑谱：它知道哪些概念相互支持，哪些旧判断被新材料修正，哪些边是噪音，哪些资料值得唤醒推理。

## 6. 非目标 / 边界

V2 不做 Hindsight 复刻，不做默认 query-time reflect，不做多租户/RBAC/团队协作，不做重型 Postgres，不做复杂 Control Plane，不做自动 mental model 洪流，不默认后台无限自学习，不做完整 Obsidian 插件生态，也不把 LLM 生成答案当检索主路径。

V2 可以有 Tauri UI，但 UI 是座舱，不是发动机。V2 可以借鉴 HelixNotes/Obsidian 的 Markdown vault、wiki link、graph view、迁移体验，但不能把 orderk 核心绑死在某个 GUI App 上。V2 可以写 Markdown，但只写经过门禁的 wiki/concept/digest 层，raw 层只追加或导入，不被 LLM 随意改。

## 7. 语言与技术栈

核心后端优先 Rust。理由不是宗教，而是 orderk 的热路径非常适合 Rust：文件扫描、frontmatter 解析、chunk、hash、索引、SQLite/Tantivy、模型 HTTP client、MCP/CLI、doctor/eval 都需要稳定、低资源、可分发、可验证。

建议 crate 切分：外部 Markdown base / adapter 管 vault、编辑、迁移和可视化 wikilink graph；orderk 内部只保留智能层切分：`orderk-index` 管 FTS/vector/search，`orderk-models` 管 cloud embedding/reranker/LLM profile，`orderk-sword-spirit` 管唯一剑灵 digest，`orderk-graph` 管概念/实体/claim/edge，`orderk-cli` 管机器入口，`orderk-api` 管本地 HTTP/MCP。

UI 层可用 Tauri 2 + Svelte/SvelteKit。这里可以参考 HelixNotes，因为 HelixNotes 已经证明 Rust + Tauri + Svelte + Markdown vault + graph + import + Tantivy search 这条路可行。但 orderk 的核心不能被 UI 主导：CLI/API/digest/eval 应先成形，UI 后接。

检索底座优先保留 orderk 现有 SQLite/FTS/vector 体系，除非 benchmark 证明 Tantivy 在目标语料上有明显收益。HelixNotes 用 Tantivy 是优点，但不是必须照搬。最高性价比路径是：先用现有 orderk index 打通 V2 digest 结构，再按实测决定是否引入 Tantivy 作为全文搜索 backend。Tantivy spike 的硬门槛是：在 30-50 条 golden queries 和一份 5K 文档语料上，top-5 expected source hit 或 MRR 相比现有 FTS5 至少提升 10%，且 p95 延迟不超过现有路径 2 倍、索引体积不超过 1.5 倍；达不到就保持 SQLite/FTS5，把工程预算留给 digest/graph。

## 8. HelixNotes 评估

HelixNotes 是值得关注的 md 文件系统、编辑器与可视化 graph 底座候选，但它不应承担 orderk V2 的语义检索核心。这里要把“底座”拆成两层：第一层是人类可直接使用的 Markdown vault、编辑器、附件、版本、Obsidian 迁移、wikilink 可视化 graph；第二层是 orderk 的 agent-first 语义索引、rerank、digest、wiki/semantic graph、score trace、golden queries 和 MCP/CLI evidence contract。茶老板本意是让 HelixNotes 尽量承接第一层，减少我们重复造 Markdown 文件系统和图谱座舱，把工程重心集中在 orderk 的第二层。

以下信息是 2026-06-04 的一次性技术侦察快照，来源为 helixnotes.com、Codeberg 仓库/API、release 页面和 docs 页面；后续如果要 fork、嵌入或兼容，必须重新验证 license、commit、release 质量、importer 行为和数据格式。已验证事实：它开源在 Codeberg，AGPL-3.0-or-later；官网标 v1.2.9；技术栈是 Svelte/SvelteKit + Rust + Tauri 2.0；Codeberg API 显示仓库创建于 2026-02-09，更新到 2026-06-03，stars 78、forks 4、watchers 12、open issues 35、release counter 30；release v1.2.9 的 Android APK 下载约 3572，Windows installer 约 930，macOS dmg 约 317。它支持 plain `.md`、vault 文件夹、多 vault、`.helixnotes/` 元数据、wiki links、graph view、Tantivy 全文搜索、版本历史、backup、AI actions、Obsidian importer。

好处是它和茶老板想要的“md 底座 + Rust/Tauri + 可视化图谱 + Obsidian 迁移”高度贴近，说明这个方向不是空想。它已经有 Obsidian importer：Settings > Import 选择 Obsidian vault，会转换 wiki links 和 image embeds；它的 vault 结构也符合“notes are plain md, metadata in hidden folder”的思路。因此，V2 可以优先评估 HelixNotes 作为 Markdown/graph cockpit：用户在 HelixNotes 里写、看、改、连 wikilink；orderk 读同一批 Markdown 文件和链接关系，建立自己的搜索索引、语义图谱和 digest audit。这样分工更清楚：HelixNotes/Noteriv-class base 管文件、标签和可视化座舱，orderk 管检索刀法、剑灵 digest 和证据链。

风险是 HelixNotes 的 graph 与 orderk 的 semantic graph 不是天然同一个东西。HelixNotes graph 主要来自 `[[wikilink]]`，表达的是人显式写出来的“这篇笔记连到那篇笔记”；orderk semantic graph 还会表达 LLM/reranker/digest 发现的 `supports/refines/contradicts/replaces/part_of/depends_on` 等证据关系，表达的是“这些概念/claim 在语义和证据上有什么关系”。前者适合作为可见、可编辑、人类友好的粗图；后者适合作为检索、rerank、主动唤醒推理的机器图。两者可以映射和互相投影，但不能无脑合并成同一张图，否则会把“人手动连的导航边”和“机器推断的语义边”混在一起，graph 会变脏。

结论：**HelixNotes 可以作为 orderk V2 的 Markdown 文件系统、编辑器、迁移器和可视化 graph 座舱候选；orderk V2 仍然拥有语义检索、semantic graph、digest、rerank、eval 和 MCP/CLI contract。** 最划算的做法不是重写一个 HelixNotes，也不是让 HelixNotes 接管 orderk 搜索，而是做一层 adapter：读取 HelixNotes vault 的 `.md`、frontmatter、attachments、wikilinks 和 `.helixnotes/` metadata；把 HelixNotes graph 作为显式 link graph 输入 orderk；orderk 另建 semantic graph，并把高置信 semantic edge 以可审计 proposal 的形式投影回 Markdown/wikilink/frontmatter，是否让 HelixNotes 可视化显示由用户批准。
## 9. 信息架构：x、y、知识库存

V2 里的 x 是已有知识存量，不只是 chunk 列表，而是由 raw、wiki concepts、entities、claims、decisions、edges、audit 共同构成的“知识库存”。y 是新增 raw 或新增文档。系统的核心不是简单 `x = x + y`，而是：

```text
x' = prune / link / distill / weight (x + y)
```

也就是说，新材料不一定全进入生效知识。LLM 负责提出“这批 y 可能改变了哪些 x”，embedding 负责找相近旧知识，reranker 负责筛掉牵强连接，规则门禁负责防污染，最后高价值部分才让 x 变厚。

V2 的知识晶核包括：稳定概念页、实体页、claim/decision 卡、边权图谱、冲突/替代关系、来源证据、黄金查询集、sword-spirit audit。它不靠一次回答显得聪明，而靠这些晶核长期增长。

## 10. Vault 文件结构建议

```text
vault/
  raw/                         # 原始材料，只追加/导入，不让 LLM 乱改
    articles/
    conversations/
    docs/
    attachments/
  wiki/
    concepts/                  # 概念页，Markdown，可审计更新
    entities/                  # 人/项目/工具/模型/组织
    claims/                    # 稳定判断、事实断言、边界说明
    decisions/                 # 架构决策记录
    queries/                   # 值得保留的高价值查询综合
  graph/
    edge-proposals/            # 中置信候选边，可人工批准
    rejected/                  # 低置信或噪音样本，供调参
  .orderk/
    index.sqlite               # 可重建：FTS/vector/chunk/source metadata
    graph.sqlite               # 可重建或半可重建：edge weights/cache
    ops.jsonl                  # 导入/索引/digest 状态
    audit.jsonl                # 谁在何时改了什么，模型/版本/来源
    profiles.json              # provider/model/dim/backend guard
    eval/                      # golden queries / baselines
```

如果兼容 HelixNotes，`.helixnotes/` 可以作为 UI metadata 层存在，但 orderk 的核心运行时不应依赖它。orderk 应能直接操作标准 Markdown vault，也应能把 HelixNotes/Obsidian vault 当输入源。

## 10.1 最小数据契约

V2 开工前必须先冻结最小 schema，不然 digest、graph、UI、MCP 会各写各的。核心对象至少包括 Source、Chunk、Concept/Entity/Claim/Decision、Edge、Proposal、Audit。Source 记录 `id/path/kind/hash/mtime/frontmatter/ingest_time/profile/raw_immutable`；Chunk 记录 `id/source_id/line_range/heading_path/text_hash/embedding_profile/vector_id/metadata`；wiki 对象记录 `id/slug/markdown_path/type/status/version/source_refs/last_digest_id`；Edge 记录 `id/from_id/to_id/type/weight/evidence_refs/created_by_digest/status/updated_at`；Proposal 记录 `id/digest_id/type/target/patch_or_edge/evidence_refs/confidence/reranker_score/decision`；Audit 记录 `id/time/actor/model_or_profile/operation/inputs_hash/outputs_hash/files_changed/rollback_ref`。

wiki 下的 concept/entity/claim/decision 统一使用 Markdown + frontmatter。初版 frontmatter 形状如下，字段可以收敛，不能隐式扩张：

```yaml
---
id: urn:orderk:concept:retrieval-depth
type: concept                         # concept | entity | claim | decision
title: "检索深度"
aliases: [retrieval depth]
tags: [retrieval, ranking]
status: draft                         # draft | growing | stable | superseded
confidence: medium                    # low | medium | high
created: 2026-06-04T00:00:00Z
updated: 2026-06-04T00:00:00Z
source_refs:
  - chunk_id: chunk_abc123
    path: raw/articles/example.md
    line_range: [12, 45]
    quote_hash: sha256:...
supersedes: []
superseded_by: []
---
```

Edge 可以落 SQLite，也可以以 JSONL 做 proposal/audit 影子；但 active edge 必须能从 wiki frontmatter/source refs + audit.jsonl 重建。初版只允许 6 种语义边：`supports`、`refines`、`contradicts`、`replaces`、`part_of`、`depends_on`。`mentions` 不作为语义边，只做 FTS/vector 召回信号；`analogous_to` 和 `causes` 太容易被 LLM 主观泛贴，先留在 reasoning mode 的临时表达里，不写入 graph。

```json
{
  "id": "urn:orderk:edge:e001",
  "source": "urn:orderk:concept:retrieval-depth",
  "target": "urn:orderk:concept:reranking",
  "type": "depends_on",
  "weight": 0.87,
  "confidence": "high",
  "evidence_chunks": ["chunk_abc123", "chunk_def456"],
  "proposed_by": "digest-v0.1-20260604",
  "proposed_at": "2026-06-04T00:00:00Z",
  "status": "active"
}
```

raw/source 层只追加或导入，不允许 LLM 原地改写；wiki/concept/decision 层的正文变化默认以 patch/proposal 形式出现；index/vector/cache 均必须可从 Markdown 与 audit 重建。Chunk 需要 `created_at/updated_at` 或等价 version timestamp，避免重新 chunk 后 digest 无法区分新旧。profile 漂移要 fail-closed：每个 chunk/vector/score 绑定 embedding/reranker profile，模型或维度变化后 doctor 标记 stale index，要求重建或分区比较，不能把不同 profile 的分数混在一起。`profiles.json` 至少记录 `profile_id/provider/model/dimension/backend/index_type/created_at/status/fallback_policy`，并为 search、digest、reasoning 分别标注默认 profile，防止不同 crate 各自猜。

## 11. 唯一后台剑灵：Digest Loop

剑灵只在 raw 入库后、或用户明确触发 `orderk digest` 时运行。它不是常驻大脑，也不是 query path 的默认环节。

流程是：首先 scanner 发现新增或变化的 raw y，记录 hash、source、mtime、frontmatter、chunk。然后 candidate retriever 用 embedding、关键词、wikilink、tag、path、title 找出 y 最可能关联的旧概念、实体、claim、decision。接着 LLM 只在受限上下文里做结构化分析，输出候选关系和候选更新，不写最终答案。候选关系类型必须小而有力，初版只保留 `supports`、`refines`、`contradicts`、`replaces`、`depends_on`、`part_of`。然后 reranker 对候选边/候选 patch 重排，结合证据强度、新旧时间、概念质量、重复度、冲突风险给出权重。最后 writer 按阈值执行：高置信边自动写入 graph/cache 和相关概念的 source mention；中置信进 `edge-proposals/`；概念正文改动默认生成 patch 供批准；低置信进入 rejected 样本。

这个机制的关键不是“自动写很多东西”，而是让每批新材料都经过一次“和旧剑谱对照”的动作。剑灵每天只做一轮受控 digest，不到处乱改，但每一轮都让知识库存更厚。

Digest 默认预算要先保守：embedding 邻居召回 top-30，BM25/topical 邻居 top-10，RRF 去重后最多给 LLM 20 个候选邻居；单次 digest job 输入不超过 8K tokens；每批最多处理 20 个新增/变更文件或 200 个 chunk；候选边每批最多 50 条，低置信自动砍到 10 条以内。超预算时不强跑，进入 proposal-only 或 queued 状态。digest 写入必须事务化：先 dry-run 生成 proposals 和 audit preview，确认后再 apply；高置信边是在 dry-run、schema 校验、预算校验都通过后自动 apply，中置信仍进入 proposal queue。崩溃后从 audit.jsonl 最后 confirmed batch resume；并发 ingest 使用文件级 lock；两条 raw 互相 contradict 时不自动裁决，生成 conflict_pair proposal 给用户批准。P3 之前还要冻结 digest prompt contract：prompt 版本、输入片段类型（raw chunk 摘要、候选邻居 frontmatter、source quote）、结构化 JSON 输出 schema、校验失败重试策略都必须进入 audit。

## 11.1 Hindsight 精量比对后的量产槽位

2026-06-04 对 Hindsight 实际源码 `/home/agent/services/hindsight/.venv/lib/python3.12/site-packages/hindsight_api` 做精量比对后，V2 量产版必须补齐以下槽位。吸收的是机制，不复制 Hindsight 的完整主脑器官。

**Budget profile**：Hindsight 用 `low/mid/high` budget 映射 thinking budget，并支持 fixed/adaptive 两种预算函数。orderk V2 要把这个机制改造成 `fast/standard/deep/digest_low/digest_standard/digest_deep/eval`，每个 profile 同时约束 candidate caps、vector top-k、BM25 top-k、graph expansion、reranker cap、LLM call cap、token cap、wall-time cap、fallback policy。预算必须可审计地进入 `thinking` / `routing` / `trace`，不能散落成 magic constants。

**Fusion / rerank**：Hindsight 的核心不是“多搜几路”，而是 semantic、BM25、graph、temporal 多路候选先用 RRF 合并，再对有限候选 rerank，并把 recency / temporal / proof_count 作为有界小 boost 调制主相关性。orderk V2 要吸收 RRF `k`、source ranks、rerank `max_candidates`、passthrough reranker guard、NaN/Inf score sanitize、provider score normalization；拒绝直接相加不可比 raw score。

**Graph expansion**：Hindsight 的 link expansion 同时利用 entity、semantic kNN、causal link，并用 per-entity cap、timeout fallback 防止高 fanout 爆炸。orderk V2 不复制 observation/mental-model 图谱，但要保留 `entity/semantic/causal/temporal` 四类可解释 expansion signal，默认 compact trace，只有 debug/high trace 才记录全量 visit/prune。

**Tag / scope isolation**：Hindsight 的 `any/all/any_strict/all_strict` tag 语义很关键，strict 模式会排除 untagged，consolidation 也按 exact tag scope 分批。orderk V2 要明确 untagged 是否可见，digest/proposal 要支持 scope isolation，防止一个项目的低置信边串到另一个项目。

**Trace contract**：Hindsight 的 SearchTracer 会记录 query info、retrieval phase、RRF merge、rerank、score components 和 final summary。orderk V2 要冻结 `trace_level=off|compact|full`：compact 至少记录 query slots、budget、retrieval arm counts、source ranks、score components、fallback/warnings；full 才记录 graph visit/prune 细节。

**Proposal governance**：Hindsight consolidation 对 LLM action 有硬门禁：update/delete 的 target 必须来自 evidence set；LLM batch 失败会 adaptive split；duplicate updates 会去重并合并 source ids；scope 达上限后只允许 update/delete，不允许 create。orderk V2 的剑灵 proposal 也必须有 evidence-set gate、adaptive split、duplicate-action dedupe、scope cap、append-only audit 和 redaction policy。

**Fallback/status schema**：所有 provider 调用必须区分 `not_called`、`called`、`called_unparseable_fallback`、`called_failed_degraded`、`called_timeout_degraded`。MiniMax 只返回 thinking block 不能被粉饰成 typed decision 成功；fallback 也要有 proposal-only / skipped / queued 的明确状态。

### 11.1.1 Absorb / Adapt / Reject 决策表

**Absorb，直接吸收机制**：
- Budget profile：吸收 Hindsight 的 `low/mid/high` 预算观，但改成 orderk 语义的 `digest_low/digest_standard/digest_deep/eval`；所有 candidate cap、reranker cap、LLM cap、fallback threshold 必须来自 profile。
- RRF + bounded boosts：保留现有 RRF 思路，补齐 source rank trace、NaN/Inf sanitize、reranker score normalization，拒绝直接混加不可比 raw score。
- Search/digest trace：吸收 SearchTracer 的阶段性 trace contract，先落 compact trace 到 JSON，再逐步扩 full trace。
- Evidence-set gate：LLM proposal 只能引用候选 evidence set 内对象；不存在的 target/action 一律丢弃或进入 rejected 样本。

**Adapt，改造成轻量形态**：
- Hindsight graph expansion 不照搬全量 entity/observation/mental-model 图谱，只改造成 sidecar semantic edge + link graph adapter + bounded graph boost。
- Hindsight consolidation worker 不照搬重队列，只改造成单后台剑灵 digest run：事务化 sidecar、proposal queue、audit resume、scope cap。
- Hindsight tag_groups / strict scope 改造成 vault/project/profile scope：默认不跨 scope 自动生效，untagged visibility 明确写入 profile。
- Reflect/reasoning 改造成主动唤醒推理：只在低置信、冲突、统综/架构意图时触发，不进入默认 search path。

**Reject，明确不搬器官**：
- 不复制 Hindsight 的主脑 bank、operation queue、mental model 刷新洪流、默认 reflect answer path。
- 不把 Hindsight 的 Postgres/worker/control-plane 作为 orderk 核心依赖。
- 不让 LLM 直接写 raw/source Markdown，不让 MCP 远程自开写权限。
- 不把 provider fallback 静默伪装成成功；fallback 必须显式记录并进入评测。

### 11.1.2 量产车级别 P0 门禁

下赛道前先把 P0 门禁从“能跑”升级成“能控”：
1. `orderk sword run` 支持 budget profile 与 trace level，并在 `thinking` / audit / report 中记录实际使用的 caps、阈值和 fallback policy。
2. 所有 active digest magic constants 都必须归一到 budget profile：candidate multiplier/min/max、per-source lexical/embedding/reranker cap、LLM candidate cap、fallback threshold。
3. 50-query / 50-digest fixture bench 输出固定 JSON：base、sword、Hindsight reference 三方的 top-k hit、MRR、latency、RSS、proposal precision proxy、fallback distribution。
4. 真实 3,713-md vault active run 至少完成一轮 bounded digest，不改 raw，sidecar 可读，secret marker 为 0；若 provider 慢/失败，必须报告 degraded 状态而不是假成功。
5. release gate 之前必须有独立审计：边界、secret、budget/trace、fallback、raw immutability、bench 脚本都要过。

### 11.1.3 本轮 real-battle 实测边界

本轮实测先暴露了两类边界，再在 `0.1.14` 修正版把“全程赛道”固化为可重复 gate：

- **全库 remote embedding 旧失败证据**：早期对 `/home/agent/obsidian-vault` 的全库 remote embedding/index 尝试超过 10 分钟轻量阈值仍未完成；停机证据记录在 `/tmp/orderk-sword-full-vault-aborted-evidence.json`。该证据只说明旧路径过重，不能当作全库通过。
- **代表性 50-doc sample 旧边界**：`scripts/sword_real_vault_bench.py` 只索引真实 3,713-md vault 的 50-doc sample。旧 `0.1.13` 数据可支持“sample 赛道 MRR/near-neighbor 改善”，但因为 Sword top1 曾从 34/50 小退到 33/50，不能写成“3,713-md 全库量产达标”。
- **Search guard 回归修复**：sample bench 暴露 `orderk sword search` 的 sidecar boost 曾在 chunk 级排序里污染 topN；`0.1.14` 通过 top1 non-regression 回归测试锁住修复：无 sidecar boost 时保留 base 排序，sidecar boost 只在 evidence overlap 时小幅生效。
- **0.1.14 全库 active gate 通过**：`scripts/sword_full_vault_active_gate.py` 每次先 rebuild 当前 `orderk` binary 并校验版本，再复制真实 vault 运行 `orderk sword run --thinking active --max-files 3713`。最新证据 `/tmp/orderk-sword-full-vault-active-gate-0.1.14-final-20260604T182359/summary.json`（sha256 `44a42773628ac4607e3d3b6f3e9187773e171bbc983d31d6505a1c303128e3d5`）：binary `0.1.14`，source/source considered/scanned `3713/3713/3713`，Qwen3 embedding `embedded_count=3713`，Qwen3 reranker `reranked_count=24`，MiniMax typed LLM `llm_invocation=called` / `llm_calls=2`，`fallback_invocation=not_used`，raw unchanged `true`，sidecar `neighbors=24/proposals=9/rejected=3/audit=1/report_exists=true`，wall `6:08.98`，max RSS `440368 KB`，warnings `[]`。

## 12. 检索链路

快查档：本地 FTS/BM25 + path/title/tag/heading，完全不调用 LLM，目标是毫秒到低秒级响应，适合找文件、旧句子、旧配置、明确关键词。

标准档：BM25 + vector + RRF + light metadata boost，必要时 reranker 精排，返回证据、来源、score breakdown、chunk 上下文。这里依然不默认 LLM 生成答案，重点是找到对的证据。

深查档：在标准档基础上做 graph expansion，从命中的概念/claim/decision 扩展到相关边，再 rerank。只有当用户要求判断、复盘、架构、统综，或者系统检测到命中低置信/冲突/结果稀疏时，才主动唤醒 reasoning mode，让 LLM 基于证据综合。

这和 Hindsight 的区别是：Hindsight 可以在 recall/reflect 中现场组织理解，orderk V2 更强调“平时消化好，查询时跑得快”。

## 12.1 Search Result Contract

`orderk search --json` 必须返回可审计结果，而不是只给文本摘要。最小字段包括：`query`、`mode`、`reasoning_triggered`、`trigger_reason`；`results[]` 里至少有 `source_id/path/line_range/title/snippet/chunk_id`；`scores` 里至少拆出 `bm25/vector/rrf/reranker/metadata_boost/graph_boost/final`；`evidence` 里要有 `source_refs/matched_terms/activated_edges`；`profile` 里要有 `index_profile/embedding_profile/reranker_profile`；`latency` 里要有 `scan_ms/retrieval_ms/rerank_ms/graph_ms/llm_ms`；`warnings` 里要能表达 `stale_index/profile_mismatch/undigested_raw/conflicting_claims`。这些字段跨 mode 始终存在，不适用时填 `null` 或 `0`，避免 MCP/脚本消费者在不同模式下遇到字段消失。reasoning mode 只能基于这些 evidence 字段综合，不能绕过检索链路直接让 LLM 作答。

## 13. 主动唤醒推理条件

V2 需要“能主动唤醒推理”，但不能每次都推理。触发条件建议包括：用户显式说“判断/复盘/统综/架构/取舍/解释为什么”；top-k 结果分数低或相互冲突；命中多个互斥 claim；新材料尚未 digest；query 覆盖多个高层概念；用户问的是“应该怎么做”而不是“在哪里”。

唤醒后也不应变成聊天机器人，而是返回：用了哪些证据、哪些关系被激活、推理结论是什么、置信度和边界是什么、是否建议生成新的 wiki/decision patch。

Reasoning guardrail：默认搜索和标准搜索不调用 LLM；单 vault 每小时最多 10 次 deep reasoning 触发；单次 reasoning 输出 cap 4K tokens；输出必须结构化为 `evidence_used/relations_activated/conclusion/confidence/boundary/suggested_patch`；reasoning 结果不直接写入任何 wiki/graph，只能生成 proposal 或 decision patch，仍走候选/审批/audit 流程。P0 要用 5-10 条边界样本标定触发阈值，初版可从 `top1_final_score < threshold`、`conflict_pair >= 1`、`cross_concept_hits >= N`、用户 query intent 命中“判断/取舍/复盘/架构/统综”等规则开始，不允许运行时“试试看”。

## 14. 功能需求

CLI 至少包括：`orderk vault init`、`orderk ingest <path>`、`orderk index`、`orderk search`、`orderk digest`、`orderk graph explain`、`orderk proposals list/approve/reject`、`orderk doctor`、`orderk eval run`、`orderk status --json`。

MCP/API 初期只读为主：`search`、`get_source`、`explain_result`、`graph_neighbors`、`status`、`doctor`、`list_tags`、`list_concepts`。写入类工具默认不暴露，或者需要显式 allowlist：`ingest_raw`、`run_digest`、`approve_proposal`。

Tauri UI 只做座舱：搜索、证据预览、source 行号、图谱邻居、digest 队列、候选边审批、provider/profile 状态、golden query 质量、Obsidian/HelixNotes 迁移向导。不要第一版做完整编辑器、插件市场、多端同步和复杂知识控制台。

Proposal 审批在 UI 之前也必须可用。P3/P4 期间 CLI 要支持 `orderk proposals list --json`、`orderk proposals show <id> --diff`、`orderk proposals approve <id>`、`orderk proposals reject <id> --reason`、`orderk vault rollback <snapshot_id>`。审批后不直接改 raw，只 apply wiki/graph patch 并追加 audit；proposal 超过预算或积压超过 500 条时 digest 自动回压，暂停低置信候选生成。MCP 写工具默认关闭，allowlist 只能由本地 CLI/profile policy 修改，不能通过远程 MCP 自改权限；scheduler/子代理默认只读，除非茶老板明确授权 run_digest 或 approve_proposal。

## 15. 质量门禁和评测

V2 必须有黄金查询集。每次改 retrieval/digest/graph 都跑同一组 query，记录 top-k overlap、MRR、expected source hit、噪音率、无关边比例、p50/p95 延迟、模型调用数、token 成本。没有评测，所谓“更聪明”就是感觉。

剑灵要有单独评测：给一批新增 raw，检查它是否能找对旧概念邻居、是否生成合理候选边、是否把低价值边压下去、是否不污染 mature concept。graph 质量用 edge precision、orphan ratio、contradiction surfaced、proposal approval rate 来衡量。

运行健康要有 doctor：检查 raw 可读、index profile 匹配、embedding dim 一致、provider 可用、graph cache 可重建、audit 连续、secret 未泄露、MCP 只读边界、UI metadata 不污染 raw。

硬验收不能只写指标名，要先跑 V1 baseline，再比较 V2。P0 固定 50 条手工 golden queries 和 50 条 digest-loop fixture，每条带 expected sources、expected concept/edge、故意冲突样本；fixture 初版用 JSONL，至少包含 `id/query/expected_sources/expected_chunks/expected_concepts/expected_edges/intent/conflict_fixture/notes`。P1 要求删除 DB 后 `orderk index` 产出的 source/chunk hash 100% 一致；P2 要求 top-5 expected source hit ≥ 0.85、MRR ≥ 0.70、top-1 expected source ≥ 0.55，且相对 V1 baseline 不回退，若 V1 baseline 已高于绝对阈值则以“不回退 V1 + 有可解释收益”为准；无 LLM 搜索 p50 ≤ 80ms、p95 ≤ 400ms，含 reranker 深查 p95 ≤ 2.5s。P3 digest 要求人工抽查 50 条候选边 edge precision ≥ 0.80、contradiction surfaced ≥ 70%、raw 污染事故为 0；P4 proposal approval rate ≥ 0.50、orphan ratio ≤ 0.15；P6 无触发条件下 LLM 调用数必须为 0。每次 ingest/digest 的模型调用数、tokens、耗时和成本进入 audit，单批 digest token 成本超过预算时自动降级为 proposal-only 或 skip。

## 16. 统综治理线

按统综心法，V2 不是堆功能，而是调和快准狠稳。观：确认当前用户真实需要是“md 底座 + 成长型检索”，不是“复刻 Obsidian/Hindsight”。辨：旧 orderk 病机是线性，Hindsight 病机是反思过热。和：只设一个消化点，把反思集中到 digest。治：先做最小可验证 digest loop，不先做大 UI。验：golden queries、graph proposal precision、resource baseline、doctor。化传：每次剑灵提案失败都变成 rejected 样本、eval fixture、gate 或 reference。

治理边界：候选/生效分离；不可变核心是 raw 保真、secret surface、read-only 默认、profile guard；可变外围是 UI、graph 视图、edge 类型、阈值、reranker 策略。剑灵机制只许变厚知识，不许越权改 raw 或扰民通知。

## 17. 路线图

P0：冻结本 PRD、定义一句话定位、建立 50 条 golden queries 和 50 条 digest fixtures、做 HelixNotes/Obsidian 迁移样本评估、写技术 spike plan。成功标准：定位清楚，非目标清楚，评测先于实现。

P0b：V1 审计基线与迁移设计。先用同一组 golden queries 对 V1 现有系统跑 baseline，记录 MRR、top-k overlap、expected source hit、p50/p95 延迟、chunk 命中数、DB/index size，作为 V2 每阶段对照。然后输出 V1→V2 迁移设计：明确 V2 vault 与 V1 vault 的结构差异、`orderk vault migrate` 命令行为、旧 index.sqlite 是复用还是重建、迁移中断如何回滚、CLI/JSON/DB schema 哪些是 breaking change。迁移三态建议是 compatibility（V1 index 可读，V2 shadow index 并行）、shadow（V2 digest 后台跑，只生成 proposal 不写入）、cutover（V2 唯一路径，V1 index 转只读快照）；每态至少跑 7 天或一轮完整 golden queries，偏差 ≤ 5% 才进下一态。

P1：Rust Markdown vault core。完成 raw 扫描、frontmatter、chunk、hash、source citation、SQLite/FTS 现有兼容、status/doctor。成功标准：不用 LLM 也能快查，index 可重建，并完成 Tantivy spike 的 go/no-go 决策。crate 拆分顺序从现有 V1 reality 出发：先在当前 `orderk-core/orderk-cli` 内实现契约和 gate，再按 seam 抽 `vault/index/models/digest/graph/api/tauri`，不要为了目录好看提前拆 8 个 crate。

P2：标准检索。完成 embedding profile guard、vector search、RRF、reranker 精排、score breakdown、query JSON contract。成功标准：golden queries 有客观收益，token/延迟可控。

P3：唯一剑灵 MVP。完成 digest 输入/候选邻居/LLM structured proposal/reranker edge scoring/proposal queue/audit。成功标准：新 raw 能产生合理候选边和概念补充，但不会污染 raw。

P4：wiki/graph 晶核。完成 concepts/entities/claims/decisions markdown cards、edge weights、冲突/替代关系、graph explain。成功标准：x 确实因 y 变厚，而不是只多了 chunk。

P5：座舱 UI。Tauri/Svelte 展示搜索、证据、graph、digest proposals、doctor/eval，不做复杂编辑器。HelixNotes 作为 UI/迁移参考，必要时实现兼容 adapter。

P6：主动唤醒推理。基于触发条件进入 reasoning mode，输出证据链、关系激活、结论、边界、是否建议写入新 decision/concept patch。

冷启动策略：空 vault 或 raw 少于 20 篇时不跑 digest，UI/CLI 显示“x 还在蓄力，先用快查档”；首次 ingest 达到阈值后才解锁 digest。这个 20 篇是 P0 待标定阈值，不是永久规则；如果单篇 raw 很长或 source quality 很高，可由 chunk count/expected concept count 替代。回滚策略：`vault/snapshots/` 按 digest 批次号保存 wiki/graph patch diff，`orderk vault rollback <snapshot_id>` 只回滚 wiki/graph，不回滚 raw；`audit.jsonl` 永远 append-only，回滚只追加一条 rollback audit，不删除历史。

## 18. 风险清单

最大风险一：剑灵膨胀成 Hindsight。防线是只保留一个 digest 点，query 默认不 reflect，写入候选/生效分离。

最大风险二：图谱噪音越滚越大。防线是 reranker 阈值、edge 类型白名单、rejected 样本、approval rate、orphan/noise eval。

最大风险三：GUI 带偏核心。防线是 CLI/API/eval 先成形，Tauri 只做座舱，HelixNotes 只借鉴不绑死。

最大风险四：Markdown 被自动污染。防线是 raw append-only、concept patch 审批、audit、snapshot/rollback。

最大风险五：V2 变重但价值不足。防线是每个新增器官必须回答“它是否让 x 变厚、检索更准、推理唤醒更少但更准”。如果只是好看或聪明感，砍掉。

最大风险六：外部模型依赖导致成本、延迟、隐私和可用性波动。embedding、LLM、reranker 任一 provider 下架模型、改价、限流都会影响 digest 或 reasoning。防线是本地检索永远可用，profile/model/dimension 写入 audit，provider fallback 只用于 digest/reasoning 非热路径，预算超限时进入 proposal-only/offline degraded mode。

最大风险七：embedding/reranker profile 漂移导致历史向量和分数不可比。防线是每个 chunk/vector/score 绑定 profile；profile 变化后 doctor 标记 stale index，要求全量重建或分区比较；search result contract 必须暴露 profile_mismatch warning。

最大风险八：V1→V2 迁移阻断已有 workflow。防线是 P0b 先做迁移设计和 V1 baseline；P1 保留 V1 CLI 兼容模式或 side-by-side 运行；迁移后重新跑 golden queries，结果偏差超 5% 不 cutover。

## 19. 结论

orderk V2 可以比 V1 重，但必须重在“成长价值”，不能重在“产品器官”。它应该从被动检索刀升级成有剑灵的宝具：平时快查不打扰，后台消化让知识晶核持续变厚，关键问题主动唤醒推理。最高性价比的快准狠稳平衡点是：**Rust core + Markdown truth + local search + cloud semantic/rerank + one digest loop + Tauri cockpit + strict governance gates**。

最终定义可压成一句：**orderk V2 是一把 Markdown-first、Rust 驱动、带单一后台消化剑灵的智能检索宝具；它不做 Hindsight 那种全生命周期主脑，而是在保留快查刀锋的同时，让 raw 每次入库都把旧知识知识库存变厚。**
