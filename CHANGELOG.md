# Changelog

## 未发布 · Phase 1「Body MVP」进行中

### 新增
- **状态 → 动画映射**（roadmap 1.6）：Body 订阅 Core 的 `pet:state`，按 `CLIP_FOR` 表切到该活动的循环动画（idle→default、working→workone、studying→study、break→idel、sleeping→sleep、playing→playone），心情同步换到对应心情的变体。载荷用 zod 校验，不合法只 warn 不崩。摸头/提起进行中收到新状态不打断，等交互结束自然切过去；非 idle 活动不再乱插空闲小动作。
- **假状态源** `debug_set_pet_state`（Rust）：手动推一个 `pet:state` 给 Body，用来调映射；Phase 2 真状态机上线后删掉。浏览器预览里对应 `window.dispatchEvent(new CustomEvent('pet:state', { detail }))`。
- **触摸交互**（roadmap 1.3）：移植原版 `Main.xaml.cs` 的鼠标语义——短按（< 500 ms）命中头/身体区域 → 摸头 / 摸身体；长按命中提起区 → 挣扎 ×3 → 静止循环，窗口跟住光标；松手 → 当前段播完落地回默认。判定用松手那一刻的光标位置，与原版一致。
- **`pet.json` 契约** `packages/shared/src/profile.ts`：把 `vup.lps` 平铺的 `happy_px` / `nomal_px` … 收回成 `Record<Mood, Rect>`；触摸区域坐标统一在 500×500 逻辑参考系。
- `AnimationPlayer.playStep()`：只播一个段落并回调，供外部状态机逐段编排（提起就是这么拼的）。

### 变更
- 帧缓存从「最多 40 个 clip」改为 **192 MB 字节预算**。按 500×500 RGBA 估算，40 个 clip 最坏能到 ~400 MB，超出 Phase 1 DoD 的 300 MB 上限。
- `namesFor()` 改为按心情就近取名字。原版的名字表心情无关，摸身体有 1/4 概率播到只在 `ill` 下画过的动画（心情正常却是一副病容）。
- 双击摸头的演示替换为真实的区域命中；按住拖窗口替换为「提起」交互。
- `index.html` 补 favicon，开发服务器不再 404。

## v0.0.1 · 2026-09-17 · Phase 0「方案 + 脚手架 + 第一帧」

这是 VPet 从「桌宠」转向「有身体的 Personal Agent」的第一个版本。它还不会说话、没有大脑——只是把身体从 C#/WPF 搬到了 Tauri 2 + React，并把整个产品和技术方案定了下来。

### 新增
- **技术方案** `docs/`：产品定义与边界、能力地图、系统架构、Brain/记忆/RAG、Body/资产、集成、路线图、待确认问题（9 篇）。
- **资产管线** `scripts/build-assets.mjs`：移植原版 `PetLoader.LoadGraph` + `GraphInfo` 的目录推断规则，把 6181 帧 1000×1000 PNG（836 MB）转成 500×500 WebP（135 MB）+ `manifest.json`（609 段动画、438 个 type/name/mood/animat 键）+ `pet.json`（触摸区域、工作、移动规则）。增量、可重入。
- **Body（React + Canvas）**：三段式 start/loop/end 播放器，随机变体，心情降级，ImageBitmap 预解码 + LRU；呼吸 → 随机 idel 小动作；双击摸头；按住拖动窗口。
- **Core（Rust / Tauri 2）**：透明·置顶·无边框·不占任务栏的宠物窗口，自动落到屏幕右下角；托盘菜单（显示/隐藏、退出）；`app_version` 命令。
- **Monorepo**：pnpm workspace；`packages/shared`（zod 契约：PetState / Manifest）、`packages/brain`（占位）、`apps/desktop`。

### 变更
- 原 C#/WPF 工程整体移入 `legacy/`，只作参考；不再构建。
- 动画资产移到 `assets-src/`；构建产物 `apps/desktop/public/pet/` 不进 git。

### 已知限制
- 没有对话、没有记忆、没有工具——见 `docs/07-roadmap.md` Phase 1–3。
- 窗口整体不穿透鼠标（Phase 1 做 alpha 命中）。
- 仅 Windows。
