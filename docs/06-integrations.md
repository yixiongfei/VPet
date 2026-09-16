# 06 · 集成：知识库 / Obsidian / GitHub / 系统

原则：**每个数据源 = 一组显式 Tool + 一个 Permission scope + 一个 Observer 信号源（可选）**。LLM 永远不直接碰数据源。

## 1. 知识库 API（`obsidian-knowledge-base`，`http://127.0.0.1:5174`）

这是"我的网站"。它已经有 50+ 个端点，VPet 不重新实现任何业务，只包一层。

### 1.1 声明式工具 manifest

`apps/desktop/src-tauri/tools/kb.toml`——加一个网站接口 = 加一段配置，不写代码：

```toml
[base]
url = "http://127.0.0.1:5174"        # 知识库 Electron 版会随机挑端口 🔶待确认（Q9）：需要一个发现机制，见 1.3
scope_prefix = "kb"

[[tool]]
name = "kb_dashboard"
description = "今日学习总览：距考试天数、今日复习/新建/背词/做题数、阶段进度、错题本、里程碑"
method = "GET"
path = "/api/dashboard"
permission = "kb.read"
map_output = "pick(daysToExam, today, stages, wrongBooks[0:5], milestones[0:5])"   # 裁剪，防止撑爆上下文

[[tool]]
name = "kb_search_notes"
description = "在笔记里全文搜索（词法）。语义搜索请用 search_knowledge"
method = "GET"
path = "/api/search"
permission = "kb.read"
input = { q = "string", limit = "number?" }

[[tool]]
name = "kb_get_note"
description = "读取一篇笔记正文（路径相对 My-md/）"
method = "GET"
path = "/api/note"
permission = "kb.read"
input = { path = "string" }
map_output = "truncate(content, 2000)"

[[tool]]
name = "kb_schedule_day"
description = "某天的日程"
method = "GET"
path = "/api/schedule/day/{date}"
permission = "kb.read"

[[tool]]
name = "kb_create_event"
description = "创建一条日程"
method = "POST"
path = "/api/schedule/event"
permission = "kb.write"          # 默认 ask，每次确认
confirm_summary = "在 {date} {start} 创建日程「{title}」"

[[tool]]
name = "kb_review_queue"
description = "待复习的卡片队列（词汇 + 笔记）"
method = "GET"
path = "/api/review/cards"
permission = "kb.read"
map_output = "pick(total, due, items[0:10])"

[[tool]]
name = "kb_milestones"
method = "GET"
path = "/api/milestones"
permission = "kb.read"
```

v2 首批建议映射（读）：`dashboard` · `search` · `note` · `tree` · `schedule/day|month` · `review/cards` · `review/log` · `vocabulary/words`（进度） · `exams`（列表）· `exams/:id/marks`（错题/划句）· `milestones`。
写（每次确认）：`schedule/event`、`review`（提交复习结果）、`stickies`（便签）。

### 1.2 Observer 信号：`GET /api/stream`（SSE）

知识库已经有 SSE 流（vault 变化推送）。Core 订阅它，把事件转成 `Signal`：

```
kb:note_changed{path}  →  RuleEngine：若用户 10 分钟内改了同一学科 ≥3 篇笔记 → 可能在集中攻某个点 → 候选 nudge
kb:review_submitted    →  Mood Engine：+feeling；累计 30 张 → 候选 "夸一下"
```

这样 VPet **不需要**自己再 watch 一遍 vault 来感知"用户在做什么"；自己的 `notify` watcher 只服务于向量索引更新。

### 1.3 连接方式 `🔶待确认（Q9）`

| 方式 | 优点 | 缺点 |
|---|---|---|
| **A. HTTP 到已运行的知识库进程** | 零耦合；数据一致（网站看到什么，VPet 看到什么） | 知识库没开时 VPet 没数据；Electron 版随机端口需要发现（写一个 `.kb/port` 文件最简单） |
| B. VPet 直接读 `.kb/index.db`（只读） | 不依赖进程 | 绕过业务逻辑（复习队列、进度计算都在 JS 里）；schema 变了要跟 |
| C. VPet 自己起知识库 server 作为 sidecar | 独立 | 两个进程都想当 server，冲突 |

建议 **A + 降级**：知识库不在线时，`kb_*` 工具返回 `{ok:false, code:"upstream"}`，Brain 会说"知识库没开，我只能看笔记原文"，然后用 `search_knowledge`（VPet 自建的向量索引直接读 md）兜底。

## 2. Obsidian 文件系统（`D:\obsidian\yixiongfei\My-md`）

| Tool | scope | 说明 |
|---|---|---|
| `search_knowledge` | `obsidian.read` | 语义 + 词法混合检索（04 §4） |
| `read_note` | `obsidian.read` | 直接读文件（知识库不在线时用） |
| `obsidian_append_daily` | `obsidian.write.daily` | 追加到 `个人/Daily/YYYY-MM-DD.md`；文件不存在则用模板创建；**永不覆盖** `🔶待确认：Daily 目录放哪、用什么模板` |
| `obsidian_create_note` | `obsidian.write.any` | v3；默认 deny |

写入约定：VPet 写的内容一律包在标记里，方便你识别和批量删除：

```markdown
## 小V · 今日回顾
<!-- vpet:daily-review 2026-09-17T21:30 -->
- 专注 2h15m（5 个番茄钟，3 次任务切换）
- 复习 30 张卡；408 错题 3 道集中在「进程调度 · 时间片」
- 明天建议：先补《进程调度》第 41–58 行那一段
<!-- /vpet -->
```

注意知识库 app 的 `AGENTS.md` 规定了发布流程会 `git add My-md/`——VPet 写进 Daily 的内容会随知识库一起提交，这是期望行为（Daily 也是知识）。

## 3. GitHub

| Tool | scope | 说明 |
|---|---|---|
| `github_recent_activity` | `github.read` | 最近 24h 的 commit / PR / issue（你的账号，`🔶待确认` 哪些仓库） |
| `github_read_file` | `github.read` | v4 Pair Programmer 用 |

Observer：每 15 分钟轮询一次 events API（PAT 存 keyring）；信号 `github:commit{repo}` → Mood +；`github:silence>2h && activity==working` → 候选 nudge（"卡住了？"），但只在 L2 以上且用户没在番茄钟里。

## 4. 系统

| Tool / Signal | scope | 默认 | 说明 |
|---|---|---|---|
| `system_notify` | — | 身体机能 | Windows toast；主要还是靠气泡 |
| `system.idle` | `system.idle` | allow | `GetLastInputInfo`；空闲 >15 分钟 → 状态机可切 `Sleeping` |
| `system.active_window` | `system.active_window` | **deny** | 前台窗口标题；开启后 Core 只做关键词分类（IDE / 浏览器-文档 / 浏览器-视频 / 聊天），不把原始标题发给 LLM |
| 开机自启 | — | ask | `tauri-plugin-autostart` |

## 5. 日历 `🔶待确认（Q4）`

你的知识库已有 `schedule` 表（日/月/年视图、事件 CRUD、`examDate`）。建议 **把它当作唯一日历源**，不再接 Google/Outlook——少一个 OAuth，且日程和学习数据在同一个库里可以联动（"离考试还有 N 天"已经在 dashboard 里）。若你日常还在用别的日历，再加一个只读同步。

## 6. 汇总：v2 工具清单（首批 ~18 个）

```
身体机能（免授权）    start_pomodoro · stop_pomodoro · create_timer · cancel_timer · get_pet_state · system_notify
自身配置              set_setting · get_setting · set_permission（永远确认）· remember · forget_memory
知识库（kb.read）     kb_dashboard · kb_search_notes · kb_get_note · kb_schedule_day · kb_review_queue · kb_milestones
知识库（kb.write）    kb_create_event
Obsidian              search_knowledge · read_note · obsidian_append_daily
GitHub                github_recent_activity
```
