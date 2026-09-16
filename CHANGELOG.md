# Changelog

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
