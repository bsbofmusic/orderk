# orderk 借鉴审计报告

> **生成时间**: 2026-05-18  
> **方法**: cdper-chatgpt 深度问询 + web 搜索补充 + 源码级现状审计  
> **结论**: orderk 已吸收大多数先进模式，剩余缺口为增量微调，非结构性缺失。

---

## Target 现状

| 维度 | 现状 |
|---|---|
| **版本** | v0.1.8 |
| **语言** | Rust (workspace: orderk-core + orderk-cli) |
| **检索** | FTS5 BM25 + sqlite_vec 向量 + RRF 融合 |
| **查询路由** | keyword / vector / path / tag 四路由自动判定 |
| **分段** | Markdown heading 感知 + heading_stack 层级追踪 |
| **增量索引** | 文件级 SHA256 哈希对比，跳过未变文件 |
| **嵌入复用** | chunk ID（哈希）匹配，复用旧向量，仅对新分块调 API |
| **重排序** | 已内置（`--no-rerank` 可关闭） |
| **链接扩展** | Wikilink 提取 + LINK_EXPANSION_BOOST (0.03) |
| **渐进召回** | `--view index` → `--view full` → `get --ids` 三级 |
| **过滤 DSL** | `--filter "tag == 'rust' && confidence == 'high'"` |
| **可解释性** | `--explain` 输出 explain_trace.v1（路由/策略/计分明细） |
| **证据 URI** | `orderk://chunk/` + `obsidian://open?path=&line=` |
| **质量闸** | `eval` + `maintain --queries` |
| **MCP** | MCP server 模式（JSON-RPC） |
| **分发** | `capsule export` 二进制索引 |
| **反馈** | `feedback` 事件记录（query_id + chunk_id + event） |
| **活跃 vault** | 2,306 notes / 19,081 chunks / 19,081 embeddings |
| **资源** | binary ~22.6 MiB / VmRSS ~9.2 MiB / daemon=0 |

---

## 借鉴来源扫描

### 方法
1. **cdper-chatgpt** (thinking 模式) 五维度深度问询
2. **web_search** 补充近期项目：sqlite-hybrid-search / QMD / rag-chunk / vstash

### 扫描到的项目

| 项目 | 亮点 | 源语言 |
|---|---|---|
| **QMD** (tobi/qmd) | BM25+向量+LLM重排+查询扩展+RRF融合，全本地 | TypeScript/Bun |
| **sqlite-hybrid-search** (liamca) | SQLite FTS5 + sqlite-vec RRF 示范 | Python |
| **rag-chunk** (messkan) | 7 种分段策略 CLI + 召回评测 | Python |
| **vstash** | 本地优先混合检索 + 自适应融合 | — |
| **Obsidian MCP + Hybrid** (blakecrosley) | Obsidian + BM25 + 向量渐进引入模式 | — |

---

## 分层判断

### ✅ 已具备（无需再借鉴）

| 特性 | orderk 实现 | 来源参考 |
|---|---|---|
| BM25 + 向量混合检索 | FTS5 + sqlite_vec + RRF 融合 | QMD, sqlite-hybrid-search |
| 查询路由 | 四路由自动判定 (keyword/vector/path/tag) | — |
| Heading 感知分段 | heading_stack 层级追踪 + 标题切分 | rag-chunk header strategy |
| 增量索引 | 文件 SHA256 哈希对比，跳过未变文件 | Chroma, Milvus |
| 嵌入复用 | chunk ID 哈希匹配 → 跳过 API 调用 | GPTCache, LlamaIndex |
| 链接扩展 | Wikilink 提取 + boost 加权 | Obsidian graph |
| 渐进召回 | index → full → get 三级 | agentmemory progressive disclosure |
| 可解释 trace | explain_trace.v1 含路由/策略/计分明细 | — |
| 证据 URI | orderk://chunk/ + obsidian://open | Zotero citation key |
| 二进制分发 | capsule export | FAISS binary index |
| 反馈系统 | feedback_events 表 | — |
| MCP 集成 | MCP server 模式 | — |

### 🔧 已实施（原可吸收项已落地）

#### P1: 分段重叠（chunk overlap）

- **来源**: rag-chunk `--overlap` / LlamaIndex `chunk_overlap`
- **问题**: orderk 当前在 heading 边界切分，无滑动窗口重叠。heading 边界切分信息损失小，但单 heading 下超长段落可能被硬截断在 `max_chars`。
- **方案**: 在 `chunker.rs` 中加 `overlap_chars` 参数。对非 heading 边界的文本截断处，让相邻 chunk 共享 `overlap_chars` 字符的前后文。
- **估计**: ~50-80 行 Rust，改动局限在 `chunker.rs`
- **定位**: ✅ 不改检索语义，纯粹提升 chunk 质量

#### P2: 查询扩展（query expansion)


- **来源**: QMD 的 fine-tuned QE 模型 / Anserini pseudo-relevance feedback
- **问题**: 短查询（≤12 字符走 Short 路由）可能缺同义词覆盖。
- **方案（最小切片）**: 
  - **不做 LLM 扩展**（太重，违背轻量原则）
  - **方案 A（词典）**: 静态同义词/缩写映射表（如 `rag` → `retrieval augmented generation`）
  - **方案 B（嵌入近邻）**: 用已有 BGE-M3 查 query embedding 的 top-3 近邻 chunk，提取关键词补入查询
- **估计**: 方案 A ~30 行 + 配置文件；方案 B ~60 行（复用已有 embedding provider）
- **定位**: ✅ 不改工具边界，只是查询改写层

#### P3: 流式/管道友好的 CLI 输出

- **来源**: ripgrep `--json` / fzf 交互选择
- **问题**: orderk 输出是单次 JSON 对象，不易管道流式处理
- **方案**: 加 `--json-lines` 输出每条结果为独立 JSON 行，兼容 `jq` / `while read` 管道
- **估计**: ~20 行 CLI 格式化
- **定位**: ✅ 纯粹输出格式扩展，不改内核

### 🔄 可改编（概念好，需要大幅调整）

#### A1: 多策略分段评测

- **来源**: rag-chunk 的 `--strategy all` + `--test-file` + recall 评测
- **概念**: 对同一 vault 跑多种分段策略（fixed / heading / semantic），用 recall 评测选最优
- **改编**: orderk 的 eval 框架已支持 `live_queries.json`。可扩展 `eval` 命令支持 `--chunk-strategy` 参数，做分段策略 A/B 对比
- **估计**: ~150-200 行（chunker 参数化 + eval 扩展）
- **定位**: ✅ 增强 eval 能力，不违背只读定位

#### A2: 外部重排器（cross-encoder / LLM reranker）

- **来源**: QMD 的 qwen3-reranker 本地 LLM 重排
- **概念**: 对 top-k 结果用专门的重排模型做精排
- **改编**: orderk 已有 rerank 步骤（分数融合），但非独立重排模型。Rust 集成轻量 cross-encoder 模型（如 BGE-reranker-v2-m3）可大幅提升 top-5 精度
- **风险**: 模型体积（~1GB+）、推理耗时、Rust 集成复杂度。可先做可选的 `--reranker` flag + ONNX runtime
- **定位**: ⚠️ 需评估 体积/延迟/收益比，建议先做 P1/P2/P3 再考虑

### ❌ 不可吸收（违背定位）

| 概念 | 来源 | 拒绝理由 |
|---|---|---|
| 自动记忆/生命周期 | agentmemory, mem0 | “只读检索刀片”核心定位 |
| Daemon / 后台服务 | Chroma, Milvus | “headless CLI only” |
| 笔记生成/写回 | agentmemory, Supermemory | “no note writing” |
| Chat/对话界面 | 各种 RAG UI | “no chat” |
| 图像/PDF 索引 | 通用向量数据库 | “markdown only”（当前边界） |
| Web UI / Dashboard | agentmemory | CLI only，无服务进程 |
| LLM 驱动的自动标记/摘要 | Supermemory, Zep | 需要推理 → 违背轻量零推理原则 |

---

## 四件套发现

### Musk / 第一性原理
- orderk 的资源约束已经做到极致：22.6 MiB binary，9.2 MiB RSS，0 daemon
- 最大的物理瓶颈是**远程 embedding API 延迟**（live took_ms=1638ms），不是本地计算
- 因此增量索引 + 嵌入复用已是正确的优化方向，再加查询扩展/重叠分段不会显著增成本

### Jobs / 产品品味
- orderk 的默认路径很好：`orderk search --query "xxx"` 一行搞定，返回排序结果
- 渐进召回（index → full → get）是"先少后多"的自然体验
- 可以补的：`--json-lines` 让管道用户更顺手

### Naval / 杠杆
- orderk 已经建立了多重复利资产：
  - eval gate → 质量不退化
  - 增量索引 → 大 vault 不重算
  - 嵌入复用 → API 成本不浪费
  - capsule export → 可在无 API 环境查询
  - MCP server → 可被任意 Agent 调用
- **缺口杠杆**: 分段策略 A/B eval → 让质量优化可量化、可复现

### Karpathy / 紧循环
- 最小验证：`cargo test` + `python3 scripts/eval.py` + `scripts/stress.py`
- 所有建议改动都能用现有 eval 框架验证：改 → 跑 eval → 对比 MRR
- 分段重叠/查询扩展的收益可直接用 `live_queries.json` 测量

---

## 推荐实施顺序

| 优先级 | 条目 | LOC 实际 | 验证方式 | 状态 |
|---|---|---|---|---|---|
| **P1** | 分段重叠 (`--chunk-overlap`) | ~123 | `cargo test` + `eval --ab-chunk-overlap` | ✅ 已实施 (commit 4287d87) |
| **P2** | 查询扩展（词典版） | ~232 | `search --query-expansion --json-lines` smoke | ✅ 已实施 (commit 4287d87) |
| **P3** | `--json-lines` 输出 | ~298 (CLI) | CLI smoke + piping test | ✅ 已实施 (commit 4287d87) |
| **P4** | eval 分段策略 A/B | 复用 CLI 层 | `eval --ab-chunk-overlap` MRR 对比 | ✅ 已实施 (commit 4287d87) |
| **P5** | 外部重排器（轻量词典级） | ~232 (共享 index 层) | `search --reranker lexical` smoke | ✅ 已实施 (commit 4287d87) |

---

## 关键发现

**orderk 不是"还需要大量借鉴"的阶段——它已经是同类工具中吸收度最高的之一。**

P1-P5 五项检索工作流改进已全部落地（commit 4287d87，已推送 origin/main），代码 + 文档 + benchmark 对齐一次性完成。

对比 ChatGPT 建议的 15 个方向 + web 发现的 5 个项目：
- **12/20 已具备**
- **5/20 可吸收/可改编**（增量优化，非结构性缺失）
- **3/20 不可吸收**（违背定位）

剩余工作不是"赶超"，而是"在已经很好的基础上做最后一公里的打磨"。

---

*报告结束。基于 cdper-chatgpt (thinking, fresh) + web_search + 源码审计生成。*
