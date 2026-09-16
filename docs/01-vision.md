# 01 · 产品定义与边界

## 1. 最终身份：C — 有身体的 Personal Agent

三个候选里我们选 C，理由是你列出的每一条需求（自然语言、网站数据、私人老师、工作辅导、番茄钟、Obsidian、GitHub、主动提醒）都要求"大脑 + 记忆 + 工具"，而 A/B 只是给聊天机器人换皮。

```
A  桌宠 + AI                → 宠物是主体，AI 是玩具
B  AI 助手 + 桌宠形象       → 套皮 ChatGPT，没有记忆、没有身体状态
C  有身体的 Personal Agent  → 大脑是主体，身体是表达通道       ← 选这个
```

这个选择的直接后果：

- **产品核心模块是 Brain，不是 Body。** 路线图里 Body 只占 1 个 Phase，Brain/Memory/Tools/Proactive 占 5 个。
- **传统 Settings 页面不做。** 但保留一个"透明面板"（Inspector）：它记住了什么、有哪些权限、最近做了什么。这不是设置，是**信任的基础**——一个能看你 Obsidian 的 Agent，必须让你能随时看它的记忆和行为日志。
- **原版桌宠的游戏性（金钱、等级、商店、投喂）降级为可选。** 保留体力/心情作为身体状态，但驱动源从"投喂"改为"你的行为"。

## 2. 五个核心问题的建议答案

### ① 主动到什么程度？→ C（主动 Agent），但带三道刹车 `🔶待确认`

| 刹车 | 具体规则 |
|---|---|
| 频率上限 | 主动开口 ≤ 1 次 / 30 分钟（可对话调整）；同一话题 24 小时内不重复 |
| 安静时段 | 默认 23:00–08:00 不主动；番茄钟专注期内**只**在结束时开口 |
| 一句话静音 | "别烦我" / "安静一小时" → 立即 snooze，且这是确定性规则，不经过 LLM |

主动行为分三级，越高越保守：

```
L1 观察后发言   "你已经两个番茄钟没休息了"           ← 默认开
L2 观察后建议   "要不要我把这个问题整理成设计方案？" ← 默认开，需要你点头才执行
L3 观察后执行   自动写 Daily Review 到 Obsidian       ← 默认关，按 scope 单独授权
```

### ② 替你做事到什么程度？→ 权限阶梯，按 scope 逐级授权 `🔶待确认`

```
只读 ─→ 建议 ─→ 创建 ─→ 修改 ─→ 执行工作流 ─→ 自主完成任务
 │        │        │        │          │              │
 v1 默认  v1 默认  v1 部分  v2        v2            不做（v1/v2 都不做）
```

v1 建议的默认边界：

- **读**：Obsidian 全库、知识库 API、GitHub（你的仓库）、系统状态（空闲时长、活动窗口标题——后者默认关）。
- **写**：只写两处——(a) Obsidian 的 `个人/Daily/YYYY-MM-DD.md`（只追加，不覆盖）；(b) VPet 自己的数据目录。知识库 API 的写接口（创建日程、提交复习）需要每次确认。
- **执行**：番茄钟、计时器、通知——这些是它自己的"身体机能"，无需授权。**不执行 shell、不改代码、不 git push**（v1）。

### ③ 人格是什么？→ 需要你来定义，我先给一个可修改的模板 `🔶待确认`

人格不是一句"可爱的猫娘"，而是一份 **Character Card**（存为 `personality.yaml`，Brain 每次都读）：

```yaml
name: 小V              # 沿用原角色"萝莉斯"？还是新名字？
language: zh-CN        # 默认中文；日语/英语场景是否自动切换？
voice:                 # 说话方式
  register: 亲近但不腻，像一起自习的学长/学姐
  length: 默认 1–3 句；被问"为什么"时才展开
  emoji: 少量
teaching:              # 教学方式
  style: 苏格拉底式先问再讲；先确认你已知的，再补缺口
  on_mistake: 先指出哪里对，再说哪里错，最后给一个最小可验证的练习
encouragement:
  on_success: 具体地夸（夸做了什么，不夸"你真棒"）
  on_slump: 不评判，先问原因，再缩小下一步
proactivity: L2        # 见 ①
attitude_to_long_term_goal: 记住并偶尔回扣，但不说教
```

### ④ 能看到多少数据？→ 显式 Permission Scope，自然语言可改 `🔶待确认`

```
obsidian.read      obsidian.write.daily     obsidian.write.any
kb.read            kb.write
github.read        github.write
fs.read:<path>     fs.write:<path>
system.idle        system.active_window     system.notify
llm.cloud          llm.local
```

每个 scope 三态：`allow` / `ask` / `deny`。"小 V，以后你可以看我的 Obsidian，但不要修改" → `set_permission({obsidian.read: allow, obsidian.write.*: deny})`，而 `set_permission` 这个工具本身**永远**弹确认气泡。

### ⑤ 最终身份 → 见第 1 节，C。

## 3. 明确不做的事（Non-goals，至少 v1/v2）

- 不做多人联机、Steam 创意工坊、商店/金钱系统。
- 不做语音（TTS/STT）——留接口，Phase 8 之后再说。
- 不做通用"浏览器 Agent"随便上网；Web 访问只通过明确定义的 Tool。
- 不做跨平台（先 Windows；Tauri 本身跨平台，但系统观察那部分是 Win32）。
- 不让 LLM 直接读写文件系统；一切经过 Tool + Permission。

## 4. 成功是什么样的（验收场景）

这四个场景跑通，产品就成立了。路线图的每个 Phase 都对应其中一部分。

| # | 场景 | 涉及层 |
|---|---|---|
| S1 | "25 分钟后叫我" → 它创建计时器，25 分钟后弹气泡 + 动画变化 | Brain → Tool → Core Timer → Body |
| S2 | "我今天下午要把 X 写完，帮我安排" → 它读知识库日程 + 今日任务，提议 50/10 节奏，开始番茄钟，宠物进入 WORKING | Brain + Data + Core + Body |
| S3 | 晚上 21:30 它主动说："今天你专注了 2h15m，复习了 30 张卡，408 的错题里有 3 道是同一考点。要不要我写进今天的 Daily？" → 你说好 → 它追加到 Obsidian | Proactive + Memory + Data + Permission |
| S4 | "为什么 React 要这么设计？" → 它先问你已经理解了哪些，再结合你笔记里的相关内容讲，最后记下"用户在 X 上还不牢" | Tutor + RAG + Memory |
