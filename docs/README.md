# VPet → Personal Agent 技术方案

> 状态：**v0.2**（2026-09-17）。Phase 0 已完成并发布 v0.0.1（见 [CHANGELOG](../CHANGELOG.md)）。Q6/Q7/Q8/Q11 已确认（OpenAI 官方 key · 本地 embedding · Rust 核心 · C# 移 legacy/）。其余 `🔶待确认` 见 [08-open-questions.md](08-open-questions.md)，其中 **Q15（隐私路由）** 是下一个要拍板的。

## 一句话定义

> VPet 不再是"桌宠 + AI"，而是 **一个有身体的 Personal Agent**：
> 它有状态、有记忆、能看懂你的工作/学习环境，并通过自然语言陪你工作和学习。

身体是 UI，大脑才是产品。

## 文档索引

| # | 文档 | 回答的问题 |
|---|---|---|
| 01 | [产品定义与边界](01-vision.md) | 它是什么、不是什么；5 个核心问题的建议答案 |
| 02 | [能力地图](02-capability-map.md) | 从聊天到人格，一层一层拆出来的能力，以及哪些进 MVP |
| 03 | [系统架构](03-architecture.md) | Tauri + React + Rust 的模块边界、数据流、Tool 协议、权限模型 |
| 04 | [Brain：LLM / 记忆 / RAG](04-brain-memory-rag.md) | 模型抽象、Agent 循环、记忆抽取、向量库与 embedding 选型（**设计意图**；实现见 09）|
| 05 | [Body：动画与资产](05-body-assets.md) | 如何复用现有 6181 帧 PNG 动画，渲染器设计，状态→动画映射 |
| 06 | [集成：知识库 / Obsidian / GitHub / 系统](06-integrations.md) | 每个外部数据源怎么接、接哪些、权限如何 |
| 07 | [执行路线图](07-roadmap.md) | Phase 0 → Phase 8，每阶段的交付物和完成标准 |
| 08 | [待确认问题](08-open-questions.md) | 需要你回答的问题，每个都附了我的默认假设 |
| 09 | [记忆与检索](09-memory-and-retrieval.md) | **已落地的实现**：她记住什么、怎么找回来、语义向量与向量索引怎么接的。与 04 冲突时以 09 为准 |

## 现状盘点（基于本仓库与 `D:\obsidian\yixiongfei` 的实际内容）

**本仓库**（fork 自 LorisYounger/VPet，Apache-2.0）：

- C# / WPF / .NET 桌宠。5 个项目：`Core`（动画/状态/存档）、`Windows`（主程序）、`Windows.Interface`（插件接口）、`Solution`（存档编辑器）、`Tool`。
- 真正有价值、可以直接复用的资产：
  - `VPet-Simulator.Windows/mod/0000_core/pet/vup/` — **6181 帧 1000×1000 RGBA PNG，836 MB**，按 `动作类型/心情/变体/帧` 组织，文件名内嵌帧时长（如 `循环A_005_250.png` = 第 5 帧 250 ms）。
  - `pet/vup.lps` — 触摸区域、提起锚点、工作/学习/娱乐定义、移动规则。
  - 已有的状态模型（`Core/Handle/GameSave.cs`）：体力 / 饱腹 / 口渴 / 心情 / 健康 / 经验 / 金钱，4 种心情模式 Happy / Nomal / PoorCondition / Ill。
  - 已有的 LLM 插槽：`Windows.Interface/TalkBox.cs` 定义了 `ITalkAPI`（原版 ChatGPT 插件就挂在这里）——说明"对话"在原设计里就是一等公民，只是没有大脑。
- 本机**没有** .NET SDK、Rust toolchain、pnpm、Ollama；有 Node 24 + npm 11。

**知识库（`D:\obsidian\yixiongfei`）**：

- `My-md/`：数学 / 英语 / 408 / 图像 / 个人。
- `app/`：`obsidian-knowledge-base` v1.0.16，Vite + React 前端 + Express 后端（端口 5174）+ Electron 打包。
- 后端已经暴露了一套完整的 REST API（`/api/notes`、`/api/search`、`/api/schedule/*`、`/api/review/*`、`/api/vocabulary/*`、`/api/exams/*`、`/api/milestones`、`/api/dashboard`、`/api/stream` SSE）。
  **这就是"我的网站各种数据"——VPet 的 Tool 层可以几乎零成本地包一层。**

## 三条不动摇的设计原则

1. **对话驱动，而非设置驱动。** 配置仍然存在，但用户通过自然语言修改，不需要知道它在哪。
2. **LLM 是大脑，不是神经系统。** 计时、状态机、调度、权限校验、数据存取全部是确定性代码（Rust）。LLM 只做理解、规划、表达和调用工具。
3. **陪伴 + 反馈 + 鼓励 + 适度提醒，不做电子监工。** 情绪引擎只能正向放大，不能惩罚。
