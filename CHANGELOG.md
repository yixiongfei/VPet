# Changelog

## 未发布 · Phase 1「Body MVP」+ Phase 2 状态机

### 新增
- **宠物状态机**（roadmap 2.2）：`core/state_machine.rs` 的 `reduce(state, event)` 纯函数 + 16 个单元测试。体力/心情/饱腹/口渴随时间走，饿渴到阈值**自己去吃喝**——不需要用户说一句话，宠物就会动。心跳在 `lib.rs`：每秒一拍（数值按 1/60 分钟推进），吃喝演满 2.6 s（对齐夹心动画长度）后把数值补上、回到空闲；只在活动或心情变化时才推 `pet:state`，否则最多 30 s 同步一次。
  - 时间是从外面以 `Event::Tick { minutes }` 喂进来的，所以 reduce 不读时钟、不做 IO——「四小时后会饿」能在单元测试里瞬间验证，不用真等四小时。
  - **专注不打断**：只从 Idle / Break 触发进食，Working / Studying 时不打断，Sleeping 时不叫醒。
  - 摸头/摸身体经 `pet_touched` 命令回传给 Core，数值怎么变由状态机说了算，Body 不自己算。提起不扣心情——docs/01 的原则是情绪只正向放大。
  - 新命令：`get_pet_state`（Body 启动时拉一次，不用干等推送）、`pet_touched`、`debug_patch_pet_state`（改数值验证阈值）。原来的假状态源 `debug_set_pet_state` 随之删除。
  - **还没有持久化**：进程重启数值归零；`Ill` 要连续 3 天 PoorCondition，同样得等 roadmap 2.1 的 SQLite。
- **食物精灵**：夹心的中间那层补齐了。`assets-src/food/` 收了原版的 123 项食物（名字、eat/drink/gift、回多少饱腹/水），图转 128px WebP 共 ~520 KB。精灵按原版放进 `宽×宽` 的方盒等比内接、绕中心旋转、带不透明度，轨迹走 `FoodAnimation` 的 `aN` 关键帧。挑食物时排掉 `Drug`——那是原版用来救存档的药，`太阳系` 一口下去体力 −100，自发进食不该吃它。
- **夹心动画（双图层）**（roadmap 1.1 最后一项）：吃 / 喝 / 收礼是三层——后层宠物本体 → 食物精灵 → 前层手。播放器从「单 clip 单游标」重构成「多轨道共享一个时钟」：前后层帧数不同但总时长相同（Eat/Nomal 是 19 帧 vs 8 帧、都 2625 ms），每层按各自的累计时间表反查帧号，不会互相漂移。
- **饱腹 / 口渴进入 PetState**：`hunger` / `thirst` 两个维度，`Activity` 加 `eating` / `drinking`。这是自发行为的燃料——宠物有了不依赖用户输入、自己会动起来的理由。数值下降与阈值触发是 Phase 2 的 Rust 状态机的事，Body 只渲染。
- **鼠标穿透 / alpha 命中**（roadmap 1.2）：Body 每帧把画面缩到 48×48 读回 alpha、按位打包成 288 字节推给 Core（掩码没变就不推）；Core 每 50 ms 读一次光标，压在不透明像素上才关掉穿透，且只在结果变化时才动窗口。判定不用 touchhead/touchbody 矩形——它们只盖住头和身体，而 touchraised 是整幅 500 宽的带子，当命中区太粗。
  - **失败安全**：掩码未到 / 光标读不到 / 锁被污染，一律退到「不穿透」。穿透错了宠物就再也点不着，不穿透错了只是挡住下面一次点击。
  - 交互期间由 Body 钉住不穿透，否则拖拽会被轮询切断。
  - 托盘加「鼠标穿透」开关，作为 `docs/07` 风险表那条退路的运行时版本。
- **气泡 + 输入框**（roadmap 1.4）：贴着窗口底部的 DOM 气泡，流式逐字输出、超过 3 行折叠为「展开」、× 或 Esc 收起、按字数自动收起（3–15s）。输入框在气泡下方，全局快捷键 `Alt+V`（Rust 侧注册，占用失败只 warn 不影响启动）或双击宠物呼出，Esc 收起。说话时循环播 `say` 动画，说完回到当前活动。Phase 1 还没有 Brain，回话是写死的，但链路按异步生成器 yield 片段的形状搭好了，Phase 3 换成 ModelProvider 的 token 流即可。
- **状态 → 动画映射**（roadmap 1.6）：Body 订阅 Core 的 `pet:state`，按 `CLIP_FOR` 表切到该活动的循环动画（idle→default、working→workone、studying→study、break→idel、sleeping→sleep、playing→playone），心情同步换到对应心情的变体。载荷用 zod 校验，不合法只 warn 不崩。摸头/提起进行中收到新状态不打断，等交互结束自然切过去；非 idle 活动不再乱插空闲小动作。
- **假状态源** `debug_set_pet_state`（Rust）：手动推一个 `pet:state` 给 Body，用来调映射；Phase 2 真状态机上线后删掉。浏览器预览里对应 `window.dispatchEvent(new CustomEvent('pet:state', { detail }))`。
- **触摸交互**（roadmap 1.3）：移植原版 `Main.xaml.cs` 的鼠标语义——短按（< 500 ms）命中头/身体区域 → 摸头 / 摸身体；长按命中提起区 → 挣扎 ×3 → 静止循环，窗口跟住光标；松手 → 当前段播完落地回默认。判定用松手那一刻的光标位置，与原版一致。
- **`pet.json` 契约** `packages/shared/src/profile.ts`：把 `vup.lps` 平铺的 `happy_px` / `nomal_px` … 收回成 `Record<Mood, Rect>`；触摸区域坐标统一在 500×500 逻辑参考系。
- `AnimationPlayer.playStep()`：只播一个段落并回调，供外部状态机逐段编排（提起就是这么拼的）。

### 修复
- **`build-assets` 漏掉了一半的图层标记**：`layer` 靠名字后缀判断，而 `info.lps` 声明的名字带变体后缀（`eat_back_lay_2`），正则匹配不上。带 `layer` 的 clip 从 16 段修到 22 段。
- **`build-assets` 整个吞掉了 `FoodAnimation`**：这类条目没有 `path#` 子项，于是被当成「扫描本目录的 PNG」——而那个目录下只有子目录，什么也扫不到，食物轨迹 `a0..aN` 全丢了。现在单独处理，解析出 10 段夹心动画。
- **提起会被 Tauri 的 ACL 拦下**：`capabilities/default.json` 只授了 `allow-start-dragging`，而提起改用 `setPosition` 跟随光标。补 `core:window:allow-set-position`，移掉不再用的 `allow-start-dragging`。
- `docs/05` §4 写的「气泡位置由 pet.json 的 `say` 锚点决定」不成立——`vup.lps` 里没有这个键。按原版 `MessageBar` 的底对齐规则修正。

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
