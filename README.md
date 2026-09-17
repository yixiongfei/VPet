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

语义检索默认走本机 Ollama 的 embedding 接口（`qwen3-embedding:0.6b`），编译期不下载任何东西。
想完全离线、不依赖 Ollama 的可选 ONNX 后端：`pnpm dev:onnx`（会在编译期下载 ONNX Runtime）。

## 和她相处

- **单击**人物打开对话窗口；**按住拖动**把她搬到别处；**右键**打开设置。全局快捷键 `Alt+V` 也能呼出对话。
- 设置里可以调**显示大小**（200–800 px）、是否**始终置顶**，写她的**名字 / 背景 / 形象 / 性格 / 说话方式**，选一件**礼物**送她。
- 对话跑在本机 [Ollama](https://ollama.com) 上（默认 `qwen3.5:9b`，6.6 GB；嫌慢可在设置里换成 `qwen3:4b`），不联网、不上传。
  `scripts/start-vpet.ps1`（或双击 `启动桌宠.cmd`）会自动拉起 `.runtime/ollama` 里的服务、补齐缺的模型并启动桌宠。
  脚本默认打开 Vulkan 核显推理（`OLLAMA_IGPU_ENABLE=1`）：在 Intel Arc 核显上 9B 的首字延迟从 4.8 s 降到 1.9 s，
  且推理不再占 CPU。内存紧张（其他程序占用超过 ~20 GB）时，对话模型和嵌入模型会被 Ollama 轮流换出，回复会偶尔多等几秒。
- 觉得某句回答好就点 👍，不好就写下「你希望她怎么说」；这些样本可在设置里导出，
  用 [training/](training/README.md) 里的 LoRA 脚本训练成你自己的模型，再在设置里切换过去。
  聊天记录只是对话，**不会**自动进入长期记忆；说「记住：…」才会。

## 许可

代码 Apache-2.0（见 LICENSE）。`assets-src/` 中的角色美术归原作者，个人使用；公开分发前需确认授权。
