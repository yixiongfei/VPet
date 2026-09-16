# 02 · 能力地图（Capability Map）

从底到顶 9 层。每层列出能力、依赖、以及进入哪个版本。这张图决定了 [07-roadmap.md](07-roadmap.md) 的顺序。

```
 L9  人格 / 情绪     Personality · Mood Engine · Relationship
 L8  教练            Coach：日/周回顾、专注分析、任务切换分析
 L7  老师            Tutor：结合学习记录与笔记的因材施教 · Research Assistant
 L6  主动行为        Observer · Rule Engine · Daily Loop · Nudge Policy
 L5  规划            Planning：把目标拆成番茄钟/任务/日程，并跟踪
 L4  数据            Data Tools：知识库 API · Obsidian · GitHub · 系统状态
 L3  工具            Tool Protocol · Registry · Permission Gate · Audit
 L2  记忆            Memory：抽取 · 存储 · 召回（向量 + 全文）· 用户画像
 L1  对话            Chat：多轮 · 流式 · 模型抽象 · 气泡呈现
 L0  身体 / 神经系统  Body：动画 · 状态机 · 计时器 · 调度 · 存储
```

## L0 身体 / 神经系统（确定性核心）

| 能力 | 说明 | 版本 |
|---|---|---|
| 动画播放 | 复用 6181 帧资产，Start/Loop/End 三段式，按心情模式切换 | v1 |
| Pet 状态机 | `IDLE / WORKING / BREAK / STUDYING / SLEEPING / PLAYING` + 心情 `Happy/Nomal/Poor/Ill` | v1 |
| 身体数值 | 体力、心情、（可选）饱腹/口渴；随时间和用户行为变化 | v1（简化） |
| 计时器引擎 | 一次性 / 周期 / cron；持久化，重启恢复 | v1 |
| 番茄钟引擎 | 可配置节奏（25/5、50/10、自定义）；与状态机联动 | v1 |
| 触摸交互 | 摸头、摸身体、提起、拖动；对应原版触摸区域 | v1 |
| 窗口 | 透明、置顶、无边框、可穿透、多显示器、托盘 | v1 |
| 移动 / 爬墙 | 原版 move 规则 | v2 |

## L1 对话

| 能力 | 说明 | 版本 |
|---|---|---|
| 唤起 | 全局快捷键 / 点击宠物 → 输入框 | v1 |
| 流式回复 | token 级流式到气泡，同时 `Think` → `Say` 动画 | v1 |
| 模型抽象 | `ModelProvider`：OpenAI-compatible、Anthropic、Ollama；按任务路由 | v1 抽象，v2 路由 |
| 多轮会话 | Session 管理，自动摘要压缩 | v1 |
| 多语言 | 中文默认；日/英按上下文 | v2 |

## L2 记忆

| 能力 | 说明 | 版本 |
|---|---|---|
| 会话记忆 | 当前 session 的消息 + 滚动摘要 | v1 |
| 记忆抽取 | 每轮对话后由 LLM 抽取结构化事实（goal / preference / fact / progress） | v2 |
| 长期记忆存储 | SQLite；每条带 type / subject / confidence / source / created_at / last_used | v2 |
| 向量召回 | sqlite-vec + embedding；混合 FTS5 BM25 | v2 |
| 用户画像 | 由记忆聚合的 `UserProfile`，每次进 system prompt | v2 |
| 知识库索引 | 对 `My-md/**/*.md` 按标题分块、向量化、文件变更增量更新 | v2 |
| 记忆检视/遗忘 | Inspector 面板可查看、删除；"忘掉这件事" 工具 | v2 |

## L3 工具

| 能力 | 说明 | 版本 |
|---|---|---|
| Tool 协议 | `name / description / inputSchema / permission / executor` | v1 |
| Registry | Rust 侧注册，导出 JSON Schema 给 LLM | v1 |
| Permission Gate | scope 三态；确认气泡；`set_permission` 自然语言改 | v1 |
| Audit Log | 每次工具调用记录：谁触发（用户/主动）、输入、输出摘要、耗时 | v1 |
| 声明式 HTTP 工具 | 用 manifest 把知识库 API 映射成工具，不写代码 | v2 |

## L4 数据

见 [06-integrations.md](06-integrations.md)。

| 数据源 | 读 | 写 | 版本 |
|---|---|---|---|
| 知识库 API（5174） | dashboard / notes / search / schedule / review / vocabulary / exams / milestones | schedule event / review 提交 | v2 |
| Obsidian 文件 | 全库 | Daily 追加 | v2 |
| GitHub | commits / PRs / issues（你的仓库） | — | v2 |
| 系统 | 空闲时长、（可选）活动窗口标题 | 通知 | v2 |
| 日历 | 先用知识库的 schedule 作为唯一日历源 `🔶待确认` | | v2 |

## L5 规划

| 能力 | 说明 | 版本 |
|---|---|---|
| 目标 → 节奏 | "下午写完 X" → 若干番茄钟 + 休息，写入调度 | v2 |
| 计划跟踪 | 番茄钟完成/中断事件回写计划；偏差时提醒 | v2 |
| 日程冲突 | 与知识库 schedule 冲突检测 | v3 |

## L6 主动行为

| 能力 | 说明 | 版本 |
|---|---|---|
| Observer | 文件变更（vault）、知识库 SSE、GitHub 轮询、空闲、番茄钟事件 → `Signal` | v2 |
| Rule Engine | 确定性规则决定"要不要叫醒 Brain"，避免每个事件都烧 token | v2 |
| Nudge Policy | 频率上限、安静时段、snooze、去重（见 01 ①） | v2 |
| Daily Loop | 早：读日程/任务 → 今日计划；晚：汇总 → Daily Review → （授权后）写 Obsidian | v3 |

## L7 老师

| 能力 | 说明 | 版本 |
|---|---|---|
| Tutor | 回答前先查：学习记忆 + 相关笔记 + 复习/错题数据；先问再讲 | v3 |
| 弱点追踪 | 从对话、错题、复习记录中抽取 `learning.weak_point` | v3 |
| Research Assistant | 搜索 → 阅读 → 总结 → 关联已有笔记 → 学习计划 | v4 |
| Pair Programmer | 读 GitHub 代码解释 bug；修改/测试不做 | v4（只读） |

## L8 教练

| 能力 | 说明 | 版本 |
|---|---|---|
| 专注分析 | 番茄钟 + 任务切换次数 + GitHub 活动 → "今天真正专注 2h15m" | v3 |
| 周回顾 | 每周一次趋势 | v4 |

## L9 人格 / 情绪

| 能力 | 说明 | 版本 |
|---|---|---|
| Character Card | `personality.yaml`，见 01 ③ | v1（静态） |
| Mood Engine | 用户行为 → 宠物情绪，**只正向放大**：完成→开心、连续学习→兴奋、长时间无活动→轻声提醒（不生气） | v3 |
| Relationship | 相处天数、互动频率影响语气亲近度 | v4 |

## 版本汇总

```
v1 = L0 + L1 + L3 + L9(静态)          → 能对话、能计时、能变身体状态、有人格底色    （S1 场景）
v2 = + L2 + L4 + L5 + L6(基础)         → 有记忆、能查你的数据、能规划、会适度主动    （S2 场景）
v3 = + L6(Daily Loop) + L7 + L8 + L9(Mood) → 真正的老师和教练                          （S3、S4 场景）
v4 = 打磨、研究助手、周回顾、关系系统
```
