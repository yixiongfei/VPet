# VPet · Personal Agent

> 一个有身体、有状态、有记忆、能看懂你的工作/学习环境，并通过自然语言陪你工作和学习的 Personal Agent。
> 身体来自 [VPet-Simulator](https://github.com/LorisYounger/VPet) 的动画资产；大脑是新的。

**当前阶段：Phase 0**（方案 + 脚手架 + 桌面上会呼吸的第一帧）。完整技术方案见 [docs/](docs/README.md)。

## 目录

```
docs/            技术方案（01 产品定义 … 08 待确认问题）
assets-src/      原始动画帧（6181 帧 PNG）与 vup.lps，不进安装包
scripts/         build-assets.mjs：PNG → WebP + manifest.json + pet.json
packages/shared  TS/Rust 共用的 JSON 契约（zod）
packages/brain   Agent 编排（纯 TS，无 UI）—— Phase 3 起
apps/desktop     Tauri 2 + React：Body（前端）与 Core（src-tauri，Rust）
legacy/          原 C# / WPF 项目，只读参考
```

## 跑起来

前置：Node ≥ 22、pnpm、Rust stable（`rustup`）、Visual Studio C++ Build Tools、WebView2（Win11 自带）。

```bash
pnpm install
pnpm build:assets      # 首次约 3–5 分钟，生成 apps/desktop/public/pet/
pnpm dev               # tauri dev：桌面上出现宠物
pnpm build             # 产出 apps/desktop/src-tauri/target/release/bundle/nsis/*.exe
```

## 许可

代码 Apache-2.0（见 LICENSE）。`assets-src/` 中的角色美术归原作者，个人使用；公开分发前需确认授权。
