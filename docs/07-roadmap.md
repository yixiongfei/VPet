# 07 · 执行路线图

每个 Phase 都有**可演示的交付物**和**完成标准（DoD）**。顺序按依赖排，不按"有趣程度"排——Body 只占一个 Phase，是故意的。

时间是以"每周有几个晚上 + 周末"的业余节奏估的；如果全职会快 2–3 倍。每个 Phase 结束都 commit + tag。

```
Phase 0  决策与脚手架        ─┐
Phase 1  Body MVP              ├─ v1：能对话、能计时、身体会变（场景 S1）
Phase 2  Core：神经系统        │
Phase 3  Brain v1：对话与工具 ─┘
Phase 4  Memory + RAG         ─┐
Phase 5  Data Tools            ├─ v2：有记忆、能查数据、能规划、会适度主动（场景 S2）
Phase 6  Proactive + Daily Loop┘
Phase 7  Tutor / Coach / Mood  ─  v3：真正的老师和教练（场景 S3、S4）
Phase 8  打磨与发布            ─  v4
```

---

## Phase 0 · 决策与脚手架（约 1 周）— ✅ 已完成 2026-09-17，见 CHANGELOG v0.0.1

**目标**：把 [08-open-questions.md](08-open-questions.md) 的问题答完；仓库变成 Tauri 项目；宠物在桌面上呼吸。

| 任务 | 说明 |
|---|---|
| 0.1 拍板 | Q1–Q12 逐个确认，更新 docs 中的 `🔶待确认` |
| 0.2 仓库重组 | **已确认（Q11）**：C# 项目移到 `legacy/`；`mod/0000_core/pet` + `vup.lps` 移到 `assets-src/`；其余 mod 资产（food/theme/photo…）留在 `legacy/` 里不动，v2 后整体删除 |
| 0.3 工具链 | 安装 Rust（rustup）、Tauri 2 前置（WebView2 已随 Win11）、pnpm（corepack enable）|
| 0.4 脚手架 | `pnpm create tauri-app`（React + TS）→ 改成 monorepo：`apps/desktop`、`packages/shared`、`packages/brain` |
| 0.5 资产脚本 | `scripts/build-assets.mjs`：移植 GraphInfo 解析 + LPS 解析 + sharp 转 WebP + manifest.json + pet.json |
| 0.6 第一帧 | 透明置顶窗口里用 Canvas 循环播 `default/nomal` 呼吸动画 |

**DoD**：`pnpm dev` 启动后桌面上出现会呼吸的萝莉斯；`manifest.json` 的 clip 数量与原版加载数一致；`git tag v0.0.1`。

> 结果：609 clips / 6181 帧全部转出（0 帧失败）；`default` 3 变体、`say/shining` 3 loop 变体、`think` 5 loop 变体与原版目录一致；窗口 500×500 逻辑像素落在右下角，托盘可退出。原版 GraphCount 的精确对比留到 legacy 可编译时再做。

## Phase 1 · Body MVP（约 2 周）

| 任务 | 说明 |
|---|---|
| 1.1 AnimationPlayer | 三段式 start/loop/end、心情降级、双图层、LRU 预解码 |
| 1.2 穿透与命中 | Rust 侧光标轮询 + alpha 命中 → 切换 `ignore_cursor_events` |
| 1.3 触摸 | 摸头 / 摸身体 / 提起拖动 / 放下 |
| 1.4 气泡 + 输入框 | 流式文本气泡；全局快捷键呼出输入；Esc 收起 |
| 1.5 托盘 | ✅ 显示/隐藏 · 面板 · 送她礼物 · 鼠标穿透开关 · 退出。面板是独立 HTML 入口，只做了「状态 + 调试」一页——`/memory` `/permissions` `/audit` 等各自的数据存在了再加 |
| 1.6 状态→动画映射 | 订阅 `pet:state`（此时由一个假的 Rust 命令手动触发） |

**DoD**：能摸、能拖、能弹一句写死的话；CPU 空闲 < 2%，内存 < 300 MB。

## Phase 2 · Core：神经系统（约 2–3 周，主要 Rust）

| 任务 | 说明 |
|---|---|
| 2.1 SQLite | 🚧 rusqlite bundled + 迁移框架（`user_version` + 只增不改的 SQL 数组，二十行，不引框架）；已建 `pet_state_log`。其余表（`settings` `timers` `pomodoro_sessions` `sessions` `messages` `grants` `audit`）等各自的功能到位时再加——建一堆没人读的空表没有意义 |
| 2.2 PetStateMachine | ✅ `reduce(state, event)` 纯函数 + 16 个单元测试；秒级 tick 推进体力/心情/饱腹/口渴；饿渴到阈值自动去吃喝。时间从外面以 `Event::Tick { minutes }` 喂进来，所以「四小时后会饿」能在测试里瞬间验证，离线补算也复用同一段代码。`Ill` 要「连续 3 天 PoorCondition」，需要在流水上做跨天统计，留给 Phase 7 的 Mood Engine |
| 2.3 Scheduler | ✅ 一次性 / 周期；持久化在 `kv` 表；重启补发（过期的响一次，周期的下一次从现在起算）。**没用 tokio**——已有每秒一拍的心跳，再起一套调度只是多一个要对齐的时钟；到期判断做成纯函数，时间由外面喂。cron 等到真有按周/按月的需求再说 |
| 2.4 Pomodoro Engine | ✅ 相位机（25/5，四个一轮转长休 15，节奏可配）；`pomodoro:tick` / `pomodoro:phase`。跑着时把宠物按在对应的事情上（专注→work、休息→rest），但生理急需压得过它——番茄钟不该把人饿死 |
| 2.5 ToolRegistry + PermissionGate + AuditLog | ✅ `list_tools` / `run_tool` / `recent_audit`，固定走「查工具 → 过权限门 → 执行 → 落审计」。已建：`create_timer` `cancel_timer` `list_timers` `start_pomodoro` `stop_pomodoro` `get_pet_state` `request_action` `set_bias` `clear_bias` `list_biases` `remember` `forget_memory` `search_memory` `memory_context` `pin_memory` `memory_health` `set_permission`。`set_setting` / `get_setting` 等 settings 真有人读时再加；`system_notify` 等系统通知那条路打通再加。`Ask` 的确认气泡是 Phase 3 的 UX，在那之前一律按拒绝处理 |
| 2.6 Secrets | ⬜ 等 Phase 3 真的要用 API key 时再做——现在没有任何东西需要密钥，提前建一个空的密钥库只是摆设 |
| 2.7 命令与事件面 | 🚧 已有 `pet:state` `pet:said` `pet:prompt` `timer:fired` `pomodoro:tick` `pomodoro:phase` `tool:confirm` `audit:appended`。`agent:trigger` 等 Phase 6 的 Observer；`build_context` / `session_append` / `memory_upsert` 等 Brain 和记忆到位 |
| 2.8 服从与好感度 | ✅ 用户的要求不是命令，是一次概率判定。`affection`（好感度，天级慢变量）进 `PetState`；`obey::judge` 用对数几率模型算服从概率 `σ(基线 + 好感 + 心情 − 生理冲突 − 动作代价 − 压力)`；拒绝理由取冲突最大的那一项，一一对应固定台词。`request_action(target)` 是**唯一一处用户意志进入状态机的入口**，进来立刻降格成一次掷骰。被拒后反复施压会掉心情和好感 |
| 2.11 长期互动记忆 | ✅ 详见 [09-memory-and-retrieval.md](09-memory-and-retrieval.md)。`memory_items` 表（迁移 v3）是**唯一真相来源**，向量索引只是加速器。六种类型（profile/preference/habit/temporary_context/relationship/commitment），五项加权排序（语义 .55 + 重要性 .20 + 新鲜度 .10 + 使用频次 .10 + 置信度 .05），冲突按来源优先级消解。**推断不能当事实**——`plan_write` 对 `Inferred` 只回 `NeedsConfirm`，类型上就堵死了；敏感内容同理。软删除（行留着当审计，但检索入口按 status 过滤）。相似度是字面 + 语义的混合（见 2.12） |
| 2.12 语义检索 + 向量索引 | ✅ 本地 ONNX 句向量（`ort` + `tokenizers`，**不用 fastembed**——那会拖进 hf-hub 和 TLS 栈，还强依赖 HuggingFace 可达）+ sqlite-vec `vec0` 虚拟表。模型完全可选：后台加载，缺了就退回字面检索，面板显示当前用哪种。融合是加权和（两路都是余弦，不需要 RRF），稠密侧的门槛是「在不在 KNN 结果里」而非绝对阈值——句向量有各向异性，绝对阈值会把所有东西放进来。换模型自动重建索引并补算 |
| 2.9 自然语言控制 | ⬜ 把中文翻译成 `request_action` / `set_bias` 的调用。三层递进：① `intents.toml` 规则表 ② char-bigram 余弦近邻（穷人的 embedding，零依赖） ③ 本地 embedding 模型兜长尾（复用 Phase 4 的 `fastembed`，不另引依赖）。**NLU 只是工具层的前端**——Phase 3 的 LLM 调的是同一组工具 |
| 2.10 偏好权重 Bias | ✅ 「多工作一点」= 给 tag 加一个带半衰期的权重（默认 120 分钟）。**不是**优先级阶梯上的新一级：它只在「作息」那层里重排 work/study/play 三条道、让正偏置越出时段，排不过「生理急需」，也排不过「到点该睡该吃」。吃喝睡不可压制（`SUPPRESSIBLE` 白名单）——「少吃点」不该把她饿死。`set_bias` / `clear_bias` / `list_biases` |

**DoD**：不开 Brain，用 Panel 里的调试按钮调用 `run_tool("create_timer", {duration:"10s"})` → 10 秒后气泡出现、宠物动画切换、`audit` 表多一行；杀掉进程重启，未到期的计时器仍会触发。

## Phase 3 · Brain v1：对话与工具（约 2 周）

| 任务 | 说明 |
|---|---|
| 3.1 `packages/shared` | zod schema：ToolDef / ToolCall / ToolResult / Event / PetState；生成 JSON Schema 供 Rust 侧校验 |
| 3.2 ModelProvider | OpenAI-compatible（含流式 + tools）→ 接你的 OpenAI 官方 key（Q6 已确认）；Anthropic 适配器后置到 Phase 7 |
| 3.3 AgentLoop | 03 §4.1 的循环；最多 6 轮；denied 回喂 |
| 3.4 ContextBuilder v1 | personality.yaml + petState + 近期消息（还没有记忆/RAG） |
| 3.5 会话管理 | `sessions/messages` 表；超过 N 轮自动摘要 |
| 3.6 Think/Say 联动 | 请求发出 → think；首 token → say；结束 → 回状态动画 |
| 3.7 Panel：会话 + 审计 | 最简表格 |

**DoD（= 场景 S1）**："25 分钟后叫我" 端到端跑通；"以后工作日提醒我至少工作 6 小时" → `set_setting` 落库；"你可以看我的 Obsidian 但不要修改" → `set_permission` 弹确认 → `grants` 表正确。`git tag v0.1.0`。

## Phase 4 · Memory + RAG（约 2–3 周）

| 任务 | 说明 |
|---|---|
| 4.0 Spike | ~~fastembed~~ ✅ 已在 2.12 做掉，但结论不同：直接用 `ort` + `tokenizers`，模型放本地目录。见 [09](09-memory-and-retrieval.md) §5.3 |
| 4.1 EmbeddingProvider | ✅ 已在 2.12 落地（`core/embed.rs`）。接口留了 `embed.json` 配置池化方式和查询前缀，换模型只换目录 |
| 4.2 sqlite-vec + FTS5 | 🚧 `vec_memories` 已在 2.12 建好；`note_chunks`（笔记分块）和 `privacy` 列等 Obsidian 索引开工时再加。FTS5 暂时用字符 n-gram 顶替——记忆是短摘要，够用 |
| 4.3 Vault Indexer | 分块规则（04 §4.2）；`notify` 增量；`embedding_model` 不一致时重建；按目录规则打 `privacy` 标 |
| 4.3b 隐私路由 | ContextBuilder：目标 provider 为云端时剔除 `local_only` 块（Q15） |
| 4.4 MemoryExtractor | extract 路由；结构化输出；`superseded_by` 链 |
| 4.5 ContextBuilder v2 | + UserProfile 视图 + 召回记忆 + 召回笔记；token 预算（04 §6） |
| 4.6 工具 | `search_knowledge` `read_note` `remember` `forget_memory` |
| 4.7 Panel：记忆页 | 列表 / 搜索 / 删除 / 导出 Markdown |

**DoD**：跨会话它记得"用户在做 VPet 重构、在学 React/Rust"；问"我笔记里关于进程调度写了什么"能引用到具体文件和行号；Panel 里删掉一条记忆后它不再提；**断网状态下索引和检索照常工作**（证明 embedding 没出网）；`个人/` 下的块从不出现在发给 OpenAI 的请求里（Panel 的请求日志可验证）。

## Phase 5 · Data Tools（约 1–2 周）

| 任务 | 说明 |
|---|---|
| 5.1 声明式 HTTP 执行器 | 解析 `kb.toml`；`map_output` 的 pick/truncate 小语言 |
| 5.2 知识库读工具 | 06 §1.1 首批 |
| 5.3 知识库写工具 | `kb_create_event`，每次确认 |
| 5.4 `obsidian_append_daily` | 标记包裹、永不覆盖 |
| 5.5 GitHub 只读 | PAT → keyring；`github_recent_activity` |
| 5.6 知识库端口发现 | `.kb/port` 或固定端口 `🔶Q9` |

**DoD（= 场景 S2）**："我今天下午要把 X 写完，帮我安排" → 它读 `kb_schedule_day` + `kb_dashboard`，提议节奏，开始番茄钟，宠物进入 WORKING。

## Phase 6 · Proactive + Daily Loop（约 2 周）

| 任务 | 说明 |
|---|---|
| 6.1 Observer | 信号源：Scheduler 事件、知识库 SSE、GitHub 轮询、系统空闲；统一 `Signal` 类型 |
| 6.2 RuleEngine | 声明式规则（TOML）：条件 → `agent:trigger`；内置：连续 2 番茄钟无休息、空闲 15 分钟、晚间回顾时间到 |
| 6.3 Nudge Policy | 频率上限 / 安静时段 / snooze / 24h 去重；"别烦我" 是确定性快捷路径 |
| 6.4 Daily Loop | 早：`morning_brief`（读日程 + 待复习 + 里程碑 → 今日建议）；晚：`daily_review`（facts 由 Core 汇总）→ 确认后写 Daily |
| 6.5 origin=proactive 权限升级 | 03 §6 |
| 6.6 Panel：规则与静音 | 看得见每条规则最近何时触发、为何没触发 |

**DoD（= 场景 S3）**：21:30 它主动发起晚间回顾，用户点"好"后 Daily 文件多出正确的一段；说"安静一小时"后一小时内不再开口。`git tag v0.2.0`。

## Phase 7 · Tutor / Coach / Mood（约 3 周）

| 任务 | 说明 |
|---|---|
| 7.1 Tutor 模式 | 意图识别 → `tutor` 路由（强模型）；prompt：先问已知、结合召回笔记 + `kb_review_queue` / 错题、最后给最小练习 |
| 7.2 学习记忆 | 从对话 / 错题 / 复习记录抽 `learning` 类记忆（弱点、掌握度） |
| 7.3 Coach | `daily_review` 加入专注分析（番茄钟 + 任务切换 + GitHub）；一周一次趋势 |
| 7.4 Mood Engine | 05 §5 的公式落地；用户行为 → 情绪，只正向放大 |
| 7.5 personality.yaml 迭代 | 按你实际使用的感受调语气 |

**DoD（= 场景 S4）**："为什么 React 要这么设计" → 它先确认你已知的，引用你的笔记，结束后 `memories` 里多一条 `learning`。

## Phase 8 · 打磨与发布

多模型路由落地 · Ollama 本地 LLM（`local_only` 路由的 `mode=local`）· 移动/爬墙动画 · 开机自启 · 安装包（NSIS）· 崩溃日志 · 性能（帧缓存、token 用量看板）· 多语言 · 语音（留接口）。

---

## 风险与对策

| 风险 | 对策 |
|---|---|
| Rust 学习曲线拖慢 Phase 2 | Phase 2 我来写主干并逐段解释（这本身就是 Tutor 场景）；你先从 `reduce` 纯函数和单元测试上手，ownership 压力最小 |
| 透明窗口穿透 + alpha 命中在 Windows 上有坑 | Phase 1 第一天就做 spike，不行就退回"整个窗口不穿透 + 缩小窗口到宠物包围盒" |
| LLM 乱调工具 / 死循环 | 6 轮上限、denied 回喂、`origin` 权限升级、审计可见 |
| 记忆抽取产生垃圾 | 一次 ≤5 条、confidence 阈值、Panel 可删、30 天 episode 过期 |
| 主动提醒变成骚扰 | Nudge Policy 全是确定性规则，且默认保守；"别烦我" 不经 LLM |
| 知识库不在线 | `kb_*` 降级到 `search_knowledge` 直接读 md |
| 资产授权 | 个人使用；分发前确认 |
