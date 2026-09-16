# 03 · 系统架构

## 1. 技术选型总览

| 层 | 选型 | 为什么 |
|---|---|---|
| 桌面运行时 | **Tauri 2** | 透明/置顶/穿透窗口、多窗口、托盘、全局快捷键都有原生支持；产物 ~10 MB；Rust 后端天然适合"确定性核心" |
| 前端 | **React 19 + Vite + TypeScript** | 你正在学的栈；动画用 Canvas 2D，不依赖 UI 库 |
| 确定性核心 | **Rust**（tokio + rusqlite） | 计时器、状态机、调度、权限、存储必须在 webview 被隐藏/节流时依然可靠运行 |
| Brain（Agent 编排） | **TypeScript**，独立包 `packages/brain`，零 React 依赖 | LLM 生态（流式、tool calling、多 provider）在 TS 最成熟；你的 TS 比 Rust 熟；纯逻辑包以后可迁到 sidecar/Rust。**已确认（Q8）：核心用 Rust，Brain 用 TS** |
| 存储 | **SQLite 单文件**（rusqlite bundled）+ **sqlite-vec** + **FTS5** | 一个文件装下记忆/会话/计时器/审计/向量/全文；事务一致；零运维。见 04 |
| LLM 接入 | `ModelProvider` 抽象；首个适配器 **OpenAI-compatible**（覆盖 OpenAI / OpenRouter / DeepSeek / Ollama / LM Studio），第二个 **Anthropic** | 不绑死模型。**已确认（Q6）：你用 OpenAI 官方 key** |
| Embedding | `EmbeddingProvider` 抽象；**默认本地**：`fastembed` crate 进程内跑 `bge-m3`（或 `multilingual-e5-base`）；备选 Ollama；云端 `text-embedding-3-small` 只作为显式可选 | **已确认（Q7）：笔记原文不出网做 embedding**；中/日/英混合内容需要多语言模型 |
| 包管理 | pnpm 12 | 与知识库 app 一致（Q12 已确认；本机 corepack 不可用，用 `npm i -g pnpm`） |

**被否决的方案**（记录下来免得以后重议）：

- Electron：知识库 app 已经是 Electron，但桌宠需要透明穿透窗口 + 长驻后台，Tauri 的内存/体积/原生窗口控制更合适。
- 全 Rust（含 Brain）：LLM SDK 生态弱，你的 Rust 还在学，会把产品进度绑在语言学习上。
- Node sidecar 跑 Brain：多一个进程、打包复杂；`packages/brain` 零 DOM 依赖已经保留了这条退路。
- LanceDB / Qdrant / Chroma：数据量（几千条笔记块 + 几千条记忆）远不需要独立向量服务；sqlite-vec 一个文件搞定。

## 2. 进程与模块

```
┌──────────────────────────────── Tauri App（单进程）────────────────────────────────┐
│                                                                                  │
│   WebView: Pet Window            WebView: Panel Window (Inspector, 按需)          │
│  ┌──────────────────────┐        ┌──────────────────────┐                        │
│  │  Body (React)        │        │ 记忆 / 权限 / 审计   │                        │
│  │  · AnimationPlayer   │        │ / 会话 历史          │                        │
│  │  · Bubble / ChatBox  │        └──────────────────────┘                        │
│  │  · TouchLayer        │                                                        │
│  ├──────────────────────┤                                                        │
│  │  Brain (TS, 无 UI)   │  ← packages/brain                                      │
│  │  · AgentLoop         │                                                        │
│  │  · ContextBuilder    │                                                        │
│  │  · ModelProvider     │ ──── HTTPS ───→ OpenAI / Anthropic / Ollama            │
│  │  · MemoryExtractor   │                                                        │
│  └───────┬──────────────┘                                                        │
│          │ invoke / listen（Tauri IPC）                                           │
│  ┌───────▼──────────────────────────────────────────────────────────────────┐    │
│  │  Core (Rust)                                                             │    │
│  │  · PetStateMachine   · Scheduler(Timer/Pomodoro/Cron)  · Observer        │    │
│  │  · ToolRegistry      · PermissionGate    · AuditLog    · RuleEngine      │    │
│  │  · MemoryStore       · VectorIndex(sqlite-vec+FTS5)    · Indexer(vault)  │    │
│  │  · Secrets(keyring)  · Settings                                          │    │
│  └───────┬───────────────────────┬────────────────────┬─────────────────────┘    │
│          │ rusqlite              │ notify(fs watch)   │ reqwest                  │
│     vpet.db                 My-md/**/*.md        KB API :5174 · GitHub API       │
└──────────────────────────────────────────────────────────────────────────────────┘
```

**边界规则**：

1. Body 只做渲染和输入；不含业务逻辑，不直接 invoke 工具。
2. Brain 只做"理解 → 决定调用哪个工具 → 组织语言"；**不持有任何计时、状态、权限逻辑**。它调用工具的唯一方式是 `invoke("run_tool", …)`。
3. Core 是唯一能碰文件系统、网络数据源、数据库的地方。所有工具的执行都在 Rust 侧（哪怕只是一个 HTTP GET），这样权限校验和审计只有一个入口。
4. Brain 被动运行：只被两类东西唤醒——用户输入，或 Core 的 `agent:trigger` 事件（主动行为）。Brain 自己没有定时器。

## 3. 目录结构

```
obsidian-pet/
├── docs/                      本方案
├── legacy/                    原 C# 项目（只读参考：状态公式、move 规则、LPS 解析）已确认（Q11）
├── assets-src/                原始 PNG 帧 + vup.lps（git 里保留，不进安装包）
├── scripts/
│   └── build-assets.mjs       PNG → WebP + manifest.json + pet.json（见 05）
├── packages/
│   ├── shared/                zod schema：Tool / Event / PetState / Memory 类型，TS 与 Rust 共用的 JSON 契约
│   └── brain/                 Agent 编排（纯 TS，无 React/DOM）
├── apps/
│   └── desktop/
│       ├── src/               React：pet 窗口、panel 窗口
│       ├── src-tauri/
│       │   ├── src/
│       │   │   ├── core/      state_machine.rs · scheduler.rs · pomodoro.rs · observer/ · rules.rs
│       │   │   ├── tools/     registry.rs · permission.rs · audit.rs · builtin/ · http_manifest.rs
│       │   │   ├── memory/    store.rs · vector.rs · indexer.rs · embedding/
│       │   │   ├── db/        migrations/ · schema.rs
│       │   │   ├── commands.rs   Tauri #[command] 集合
│       │   │   └── events.rs     发往前端的事件定义
│       │   ├── tools/kb.toml  声明式 HTTP 工具 manifest（知识库 API）
│       │   └── tauri.conf.json
│       └── public/pet/        构建产物：WebP 帧 + manifest.json（build-assets 生成，gitignore）
└── personality.yaml           Character Card
```

## 4. 核心数据流

### 4.1 用户说话（S1："25 分钟后叫我"）

```
用户输入 ─→ Body: ChatBox ─→ Brain.handleUserMessage(text)
                                  │
                                  ├─ Core.invoke("build_context")   → UserProfile + 相关记忆 + Pet 状态 + 可用工具列表
                                  ├─ ModelProvider.stream(messages, tools)
                                  │       └─ token 流 ─→ Body.Bubble（Say 动画）
                                  │       └─ tool_call: create_timer({duration:"25m", label:"叫我"})
                                  ├─ Core.invoke("run_tool", {name, input, callId, origin:"user"})
                                  │       ├─ PermissionGate.check("system.notify")  → allow（身体机能，免授权）
                                  │       ├─ Scheduler.create_timer(...) → 持久化到 vpet.db
                                  │       └─ AuditLog.record(...)
                                  └─ tool_result ─→ 第二轮生成 → "好，25 分钟后叫你。" ─→ Bubble

  …25 分钟后…

Core.Scheduler 触发 ─→ emit("timer:fired", {label}) ─→ Body: 气泡 + StateMachine → 动画 Switch_Up
                    └→ emit("agent:trigger", {kind:"timer_fired", …}) ─→ Brain 生成一句提醒（可选，走 Nudge Policy）
```

### 4.2 主动行为（S3：晚间回顾）

```
Scheduler(cron 21:30) ─→ RuleEngine.evaluate("daily_review")
                              ├─ 条件：今天有 ≥1 番茄钟 或 ≥1 次复习；未 snooze；不在安静时段
                              └─ 通过 ─→ emit("agent:trigger", {kind:"daily_review", facts:{pomodoro:…, review:…, exams:…}})
                                              │
                                    Brain.handleTrigger(facts)          ← facts 由 Core 确定性汇总，LLM 只负责表达
                                              │
                                    生成 "今天你专注了 2h15m…要不要写进 Daily？" ─→ Bubble + 两个按钮（好 / 不用）
                                              │ 用户点"好"
                                    run_tool("obsidian_append_daily", {content})
                                              ├─ PermissionGate.check("obsidian.write.daily") → ask → 确认气泡 → allow
                                              └─ 写入 My-md/个人/Daily/2026-09-17.md
```

### 4.3 关键点

- **facts 由 Core 汇总，不让 LLM 自己算**。"专注了 2h15m" 来自番茄钟表的 SUM，不是 LLM 心算。
- **origin 字段**（`user` / `proactive`）贯穿工具调用：主动行为触发的写操作权限门槛自动提高一级。

## 5. Tool 协议

`packages/shared/src/tool.ts`（zod）与 Rust 侧 `tools/registry.rs`（serde）共用同一 JSON 形状：

```ts
interface ToolDef {
  name: string                 // 蛇形：start_pomodoro / kb_search_notes / obsidian_append_daily
  description: string          // 给 LLM 看的
  inputSchema: JSONSchema7
  permission: {
    scope: PermissionScope     // 见 01 ④
    level: 'read' | 'write' | 'execute'
  }
  executor:                    // 只有 Rust 认识；不发给 LLM
    | { kind: 'builtin', fn: string }
    | { kind: 'http', method: string, url: string, mapInput?: string, mapOutput?: string }
  bodyMechanic?: boolean       // true = 身体机能（计时器/番茄钟/通知），免授权
}

interface ToolCall  { callId: string; name: string; input: unknown; origin: 'user' | 'proactive' }
interface ToolResult {
  callId: string
  ok: boolean
  data?: unknown                       // 给 LLM 的结构化结果（Core 负责截断/摘要，避免撑爆上下文）
  error?: { code: 'denied' | 'invalid_input' | 'upstream' | 'timeout'; message: string }
  sideEffects?: Array<{ kind: 'pet_state' | 'timer' | 'file' | 'network'; summary: string }>
}
```

Tauri 命令面（Brain 能调用的全部接口，刻意很窄）：

```
list_tools()                         → ToolDef[]（不含 executor）
run_tool(call: ToolCall)             → ToolResult
build_context(sessionId, userText)   → { profile, memories[], notes[], petState, recentAudit[] }
get_secret(name)                     → string   （仅 llm.* 与 embedding.* 的 key）
session_append(sessionId, message)   → ()
memory_upsert(items[])               → ()      （MemoryExtractor 回写）
```

事件面（Core → 前端）：

```
pet:state          { mode, mood, strength, feeling, activity }     状态机每次变化
timer:fired        { id, label }
pomodoro:tick      { phase, remainingSec }                           每秒（仅 Body 消费）
pomodoro:phase     { from, to }
agent:trigger      { kind, facts, policy }                           唤醒 Brain
tool:confirm       { callId, tool, input, scope }                    需要用户点头
audit:appended     { entry }                                         Panel 消费
```

## 6. 权限模型

```rust
enum Decision { Allow, Ask, Deny }

struct Grant { scope: Scope, decision: Decision, granted_at, granted_via: "nl" | "panel" | "default", note }
```

判定顺序：`bodyMechanic` → 直接 Allow；否则查最具体的 Grant（`fs.write:D:\x\y` 优先于 `fs.write`）；没有 Grant 用默认表（读 = Ask，写 = Ask，execute = Deny）；`origin == proactive` 时 `Allow` 降为 `Ask`（除 `*.read`）。

`Ask` 的 UX：气泡里出现"小 V 想读取你的 Obsidian（只读），允许 / 本次允许 / 拒绝"。选"允许"写 Grant；选"本次"不写。

## 7. 调度与状态机（Rust）

```rust
enum Activity { Idle, Working { pomodoro: Option<PomodoroId> }, Break, Studying, Sleeping, Playing }
enum Mood { Happy, Normal, PoorCondition, Ill }   // 与资产目录 Happy/Nomal/PoorCondition/Ill 一一对应

struct PetState { activity: Activity, mood: Mood, strength: f32, feeling: f32, updated_at }
```

- 状态机是纯函数 `reduce(state, event) -> state`，事件来自：用户触摸、工具调用、番茄钟阶段变化、时间流逝 tick、Mood Engine。
- Scheduler 基于 tokio，所有 job 持久化在 `timers` 表；启动时把过期 job 立即触发一次（补发），未到期的重新挂载。
- 番茄钟是 Scheduler 之上的一个状态机：`Focus → ShortBreak → Focus → … → LongBreak`，每次相位变化都 `reduce` 到 PetState 并发事件。

## 8. 安全与隐私

- API key 存 Windows Credential Manager（`keyring` crate），不落盘到配置文件；Brain 启动时通过 `get_secret` 读到内存。
- 所有外发请求（LLM/embedding）的目的地址在 `settings` 中显式列出；Panel 里可见"今天发出去多少 token、给了谁"。
- **Embedding 全部本地**（Q7 已确认）：索引阶段笔记原文不出网。但要清楚一个后果——**RAG 召回的笔记块在回答时仍会作为 prompt 的一部分发给云端 LLM**（OpenAI）。因此引入 **per-directory 隐私策略**（`settings.privacy`）：标为 `local_only` 的目录（如 `个人/`）的块只允许进入本地模型（Ollama）的 prompt，云端模型看不到；标为 `cloud_ok` 的目录正常召回。默认 `个人/` = `local_only`，其余 = `cloud_ok` （Q15 已确认）。
- `system.active_window`（活动窗口标题）默认 `deny`，因为它会把你在看什么发给云端模型；开启后 Core 只把标题做关键词匹配（如 "React 文档"）再喂 Brain，不发原文。
- 审计日志不可从 Brain 侧删除。
