# 04 · Brain：LLM / 记忆 / RAG

## 1. ModelProvider 抽象

```ts
// packages/brain/src/provider/types.ts
interface ModelProvider {
  id: string                                  // "openai" | "anthropic" | "ollama" | "openrouter" …
  stream(req: ChatRequest): AsyncIterable<ChatDelta>   // 文本 delta + tool_call delta + usage
  countTokens?(messages: Message[]): number
}

interface ChatRequest {
  model: string
  system: string
  messages: Message[]
  tools?: ToolDef[]           // 来自 Core.list_tools()
  temperature?: number
  maxTokens?: number
}
```

实现顺序：

1. **OpenAI-compatible**（`/v1/chat/completions`，SSE 流，`tools` 参数）——一个适配器同时覆盖 OpenAI、OpenRouter、DeepSeek、Ollama、LM Studio，只是 `baseURL` 不同。**已确认（Q6）：你用 OpenAI 官方 key**，Phase 3 用它跑通；Ollama 走同一个适配器（`baseURL=http://127.0.0.1:11434/v1`），这也是 Q15 隐私路由的本地端。
2. **Anthropic**（Messages API，`tool_use` block）。
3. 用不用 Vercel AI SDK（`ai` 包）？它已经把上面两者 + 流式 + tool calling 统一了，也支持传自定义 `fetch`（Tauri 的 `@tauri-apps/plugin-http` 可以绕过 webview CORS）。**建议用**，省掉自己维护 SSE 解析；`ModelProvider` 作为薄封装保留，防止绑死。

### 模型路由（v2）

```yaml
# settings.models
routes:
  chat:        { provider: openai,    model: gpt-5-mini }        # 闲聊、提醒措辞、日常问答
  tutor:       { provider: anthropic, model: claude-sonnet-5 }   # 教学、代码解释、长推理
  extract:     { provider: openai,    model: gpt-5-nano }        # 记忆抽取、分类（便宜、结构化输出）
  local_only:  { provider: ollama,    model: qwen3:8b }          # 召回结果含 local_only 目录的块时强制走这里（Q15）
```

Brain 按 `TriggerKind` / 用户意图选 route；用户可以说"教学的时候用 Claude"→ `set_setting` 工具改 route。
**隐私路由优先级最高**：ContextBuilder 发现召回集里有 `local_only` 块时，要么把这些块剔除后走原 route，要么整轮切到 `local_only` route——由 `settings.privacy.mode = drop | local` 决定，默认 `drop`（没装 Ollama 时唯一可行）。

## 2. Agent 循环

```
handle(input: UserMessage | Trigger)
  ├─ ctx = Core.build_context(sessionId, text)
  ├─ system = render(personality.yaml, ctx.profile, ctx.petState, policy)
  ├─ messages = [ ...session.recent(压缩后), retrieved(ctx.memories, ctx.notes), input ]
  ├─ loop (最多 6 轮):
  │     delta ← provider.stream({ system, messages, tools })
  │     文本 → Body.bubble   （边流边显示）
  │     tool_calls → 并行 Core.run_tool(...)；denied 的结果原样喂回让模型改口
  │     无 tool_call → break
  ├─ session.append(...)
  └─ 后台：MemoryExtractor.run(本轮消息)   ← 用 extract 路由，异步，不阻塞回复
```

约束：

- **最多 6 轮工具调用**，超出则告诉用户"这个我一次做不完"。防止死循环烧钱。
- **工具结果由 Core 裁剪**（如笔记正文最多 2k 字、列表最多 20 条），Brain 不做二次截断。
- **主动触发（Trigger）的 system prompt 额外包含 Nudge Policy**："只说一句；不重复 24h 内说过的；用户没回应就不追问"。

## 3. 记忆模型

### 3.1 记忆 ≠ 聊天记录

```
Conversation ──→ MemoryExtractor（LLM，结构化输出）──→ memories 表
                                                          │
                                        ┌─────────────────┼──────────────────┐
                                     profile            episodic            semantic
                                   （用户画像）        （发生过的事）       （知识/偏好/目标）
```

```sql
CREATE TABLE memories (
  id           TEXT PRIMARY KEY,
  kind         TEXT NOT NULL,      -- 'profile' | 'goal' | 'preference' | 'fact' | 'project' | 'learning' | 'episode'
  subject      TEXT,               -- 'VPet' / 'React 状态管理' / 'Rust ownership'
  content      TEXT NOT NULL,      -- 一句话，可直接放进 prompt
  confidence   REAL DEFAULT 0.7,
  source       TEXT NOT NULL,      -- 'chat:<sessionId>:<msgId>' | 'kb:exam_attempt:<id>' | 'user:explicit'
  created_at   INTEGER, updated_at INTEGER, last_used_at INTEGER,
  expires_at   INTEGER,            -- episode 类默认 30 天；goal/profile 不过期
  superseded_by TEXT               -- 被新记忆覆盖时指向新 id，不物理删除
);
CREATE VIRTUAL TABLE memories_fts USING fts5(content, subject, content=memories);
CREATE VIRTUAL TABLE memories_vec USING vec0(id TEXT PRIMARY KEY, embedding float[1024]);   -- 维度跟 embedding 模型走，见 §5.2
```

抽取示例：用户说"我最近想把 VPet 做成我的私人学习助手" →

```json
{ "kind": "goal", "subject": "VPet", "content": "用户想把 VPet 做成自己的私人学习助手", "confidence": 0.9 }
```

抽取规则（写进 extract prompt）：只抽**跨会话仍然有用**的东西；同 subject 的新事实用 `superseded_by` 链接旧的；一次最多 5 条；不确定就不抽。

### 3.2 UserProfile

不是一张表，而是一个**视图**：`kind IN ('profile','goal','preference','learning') AND superseded_by IS NULL` 按 `confidence * recency` 排序取前 N 条渲染成 system prompt 的一段。这样用户画像永远和记忆表一致，不会出现"改了记忆但画像没更新"。

### 3.3 遗忘

- `episode` 到期自动隐藏（不删，Panel 里可翻）。
- "忘掉我说过 X" → `forget_memory` 工具：语义搜索 → 列出候选 → 用户确认 → 标 `superseded_by = 'user:forget'`。
- Panel 可以一键导出全部记忆为 Markdown（放进 Obsidian 也行）。

## 4. 知识库索引（RAG over Obsidian）

### 4.1 为什么 VPet 自己建索引，而不是全靠知识库 API

知识库 server 已有 FTS（`GET /api/search`），但没有向量检索（其依赖里没有任何 embedding 库）。VPet 需要"语义上相关的笔记"来做 Tutor，所以：

- **词法检索**：可以直接用知识库 `GET /api/search`（省事、和网站结果一致）。
- **语义检索**：VPet 自建 sqlite-vec 索引。
- **混合**：两路结果做 RRF（Reciprocal Rank Fusion）合并，简单有效。

`🔶待确认（Q9）：也可以反过来——把 embedding 索引加到知识库 server 里，VPet 只调 API。好处是网站也能用语义搜索；坏处是耦合。我倾向 VPet 自建，先跑起来。`

### 4.2 分块

```
My-md/408/操作系统/进程调度.md
  ├─ frontmatter → metadata（tags、日期）
  ├─ 按 H2/H3 切块；单块 > 800 token 再按段落切；< 100 token 的合并到上一块
  └─ 每块保留：path、heading 路径（"操作系统 > 进程调度 > 时间片轮转"）、行号范围
```

```sql
CREATE TABLE note_chunks (
  id TEXT PRIMARY KEY, path TEXT, heading_path TEXT, line_start INT, line_end INT,
  content TEXT, content_hash TEXT, indexed_at INTEGER, embedding_model TEXT,
  privacy TEXT NOT NULL DEFAULT 'cloud_ok'      -- 'cloud_ok' | 'local_only'，按目录规则打标（Q15）
);
CREATE VIRTUAL TABLE note_chunks_fts USING fts5(content, heading_path, content=note_chunks);
CREATE VIRTUAL TABLE note_chunks_vec USING vec0(id TEXT PRIMARY KEY, embedding float[1024]);
```

### 4.3 增量更新

`notify` crate 监听 `My-md/`，debounce 2s → 重算文件 hash → 只重嵌变化的块。首次全量：以你现在的 vault 规模（几百个文件量级）估算几千块，本地 CPU 跑 `bge-m3` 约 5–15 分钟（后台、可中断、有进度）；之后增量每次几秒。

### 4.4 召回进 prompt 的格式

```
[笔记] 408 > 操作系统 > 进程调度 > 时间片轮转（My-md/408/操作系统/进程调度.md:41-58）
…正文…
```

带路径和行号，方便它说"你在《进程调度》第 41 行记过……"，也方便你点开。

## 5. 向量库与 Embedding 选型

### 5.1 向量库：sqlite-vec

| 候选 | 结论 |
|---|---|
| **sqlite-vec** | ✅ 选它。纯 C 扩展，Rust 有 `sqlite-vec` crate 可静态链接进 rusqlite；和其他表同一个文件、同一个事务；brute-force KNN 在 <10 万向量下毫秒级；支持 metadata 过滤 |
| LanceDB | 好，但引入 Arrow 生态、二进制体积 +30 MB；我们的量级用不上其列式优势。作为 Plan B |
| Qdrant / Milvus / Chroma | 需要独立服务或 Python，桌面单机应用不合适 |
| 纯内存（HNSW crate） | 要自己做持久化和一致性，反而更麻烦 |

### 5.2 Embedding：抽象 + 两个实现

```rust
#[async_trait]
trait EmbeddingProvider {
    fn id(&self) -> &str;            // "openai:text-embedding-3-small" / "ollama:bge-m3"
    fn dims(&self) -> usize;         // 1536 / 1024
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}
```

**已确认（Q7）：必须本地 embedding。** 所以默认方案改为进程内推理：

| 方案 | 维度 | 多语言 | 成本 | 隐私 | 结论 |
|---|---|---|---|---|---|
| **`fastembed` crate 进程内（ONNX Runtime）** | 看模型 | 看模型 | 0 | 完全本地，单进程 | **v2 默认**。免装任何东西；首次运行下载模型到 app data 目录；CPU 推理够用 |
| ├─ 模型 `bge-m3` | 1024 | 中/日/英俱佳，长文本 8k | | | **首选**（需确认 fastembed-rs 当前版本已收录；未收录则退到下一行） |
| └─ 模型 `multilingual-e5-base` | 768 | 中/日/英好 | | | 备选，更小更快（~280 MB） |
| Ollama `bge-m3` | 1024 | 同上 | 0 | 本地，多一个进程 | 若你之后为 Q15 装了 Ollama，可切换到它以共用 GPU |
| OpenAI `text-embedding-3-small` | 1536 | 好 | $0.02 / 1M | 出网 | 保留为显式可选 provider，默认不启用 |

**关键约束**：向量表的维度在建表时就定死。换 embedding 模型 = 新建 `*_vec` 表 + 全量重嵌。所以 `note_chunks.embedding_model` 字段必须有，Indexer 启动时对比，不一致就触发重建。`bge-m3` 与 `multilingual-e5-base` 维度不同，Phase 4 第一天先做 spike 定下来。

**代价**：`ort`（ONNX Runtime）会让安装包 +30–50 MB，首次启动多一次模型下载（~570 MB 量化版）。可接受。

### 5.3 检索流程

```
query ──→ embed(query) ──→ vec0 KNN top 20 ──┐
     └──→ FTS5 MATCH     top 20 ─────────────┼─→ RRF 合并 → 隐私过滤（目标 provider 是云端时剔除 local_only 块）
                                             │   → 过滤（同一 path 最多 3 块）→ top 6 进 prompt
```

## 6. 上下文预算

以 128k 上下文模型为基准，每轮硬预算：

```
system（人格 + 画像 + 政策）      ≤ 1.5k
工具定义                           ≤ 3k（v2 后工具变多，改为按意图动态选 ≤ 12 个工具）
近期会话（压缩后）                 ≤ 6k
召回的记忆                         ≤ 1k（≤ 8 条）
召回的笔记                         ≤ 4k（≤ 6 块）
工具结果                           ≤ 8k
─────────────────────────────────────────
                                   ≈ 24k / 轮，留足输出与多轮工具空间
```
