# 05 · Body：动画与资产

## 1. 现有资产的真实结构

路径：`VPet-Simulator.Windows/mod/0000_core/pet/vup/`，6181 帧，1000×1000 RGBA PNG，836 MB。

原版的解析规则在 `VPet-Simulator.Core/Graph/GraphInfo.cs:48-147`，是**从路径推断**而不是靠配置：

```
把相对路径按 \ 和 _ 拆成 token 列表，然后依次"吃掉"：
  1. 心情    happy | nomal | poorcondition | ill         （没有 → nomal）
  2. 类型    default | say | think | work | sleep | idel | touch_head | … （GraphType 枚举，按 token 序列匹配）
  3. 段落    a|start → A_Start   b|loop → B_Loop   c|end → C_End   single → Single（没有 → Single）
  4. 名字    剩下 token 的最后一个（去掉纯数字/以 ~ 开头的变体后缀）；没有 → 类型名
帧文件名：<任意前缀>_<序号>_<时长ms>.png   例如 循环A_005_250.png
同目录的 info.lps 可以覆盖以上任何字段（如 Eat/Happy/info.lps 把 back_lay/front_lay 声明为 Common 图层）
```

实际目录长这样（异构，深度不固定）：

```
Default/Happy/1/循环A_000_125.png              类型 default · 心情 happy · 变体 1 · Single
Say/Shining/B_2/…                              类型 say · 名字 shining · B_Loop · 变体 2
Think/Happy/A_2/…                              类型 think · happy · A_Start · 变体 2
WORK/Calligraphy/Happy/…                       类型 work · 名字 calligraphy · happy
Eat/Happy/back_lay + front_lay + info.lps      双图层（宠物在中间、食物前后各一层）
MOVE/…  SideHide_*  Raise  Pinch  Switch       移动/躲藏/提起/捏脸/切换
```

**结论**：不能手写映射表，必须把 `GraphInfo.cs` 的解析逻辑移植成一个构建脚本，一次性生成 manifest。

## 2. 资产构建脚本 `scripts/build-assets.mjs`

```
assets-src/pet/vup/**            ──→  apps/desktop/public/pet/
  ├─ 移植 GraphInfo 解析 → 每个叶子目录 = 一段动画（GraphClip）
  ├─ 读同目录 info.lps 覆盖字段（写一个 30 行的 LPS 解析器：`key#value:|` 按 `:|` 切、按 `#` 分）
  ├─ 解析 vup.lps → pet.json（touchhead/touchbody/pinch 区域、raisepoint、work 列表、move 规则、duration）
  ├─ assets-src/food/*.lps + image/ → manifest.food（123 项：名字、eat/drink/gift、回多少饱腹/水、图）
  │         图转 128px WebP（共 ~520 KB）。价格/经验/好感度是原版的养成经济，不带过来
  ├─ sharp：1000×1000 PNG → 500×500 WebP（quality 85，无损 alpha）    ≈ 836 MB → 60–90 MB
  │         （可选 --size=1000 出高清版，按显示缩放动态选）
  └─ manifest.json
```

```ts
interface GraphClip {
  id: string                   // "say/shining/happy/B/2"
  type: GraphType              // 'default' | 'say' | 'think' | 'work' | 'sleep' | 'idel' | 'touch_head' | …
  name: string                 // 'shining' | 'calligraphy' | 'default' …
  mood: 'happy' | 'nomal' | 'poorcondition' | 'ill'
  animat: 'start' | 'loop' | 'end' | 'single'
  variant: number
  layer?: 'main' | 'back' | 'front'   // 双图层动画
  frames: Array<{ src: string; ms: number }>   // src 相对 public/pet/
  totalMs: number
}
interface Manifest { pet: 'vup'; size: 500; clips: GraphClip[]; index: Record<string, string[]> /* "type/name/mood/animat" → clip ids */ }
```

验收：脚本跑完，`clips.length` 应与原版 `GraphCore.GraphsList` 加载数量一致（可以在 legacy 里加一行日志对比，或用 `VPet-Simulator.Tool` 的统计）。

## 3. 渲染器 `AnimationPlayer`

- **Canvas 2D**，单 `<canvas>`，按 clip 的 `ms` 用 `requestAnimationFrame` + 累计时间推进帧（不用 `setInterval`，避免后台节流漂移）。
- 帧解码：`createImageBitmap(await fetch(assetUrl).then(r => r.blob()))`，每个 clip 首次播放前预解码全部帧；LRU 缓存最多 ~40 个 clip（500×500×4B×~13 帧 ≈ 13 MB/clip → 上限 ~500 MB 显存，需要实测调低）。
- **三段式播放**：`play(type, name, mood)` → `start`（若有）→ `loop`（循环，直到 `stop()`）→ `end`（若有）→ 回到 `default`。与原版 `Main.Display(...)` 语义一致。
- **心情降级**：请求 `happy` 没有时按 `happy → nomal → poorcondition → ill` 顺序找最近的（原版 `FindGraphs` 也这么做）。
- **双图层（夹心）**：吃 / 喝 / 收礼是三层——`back`（宠物本体）→ 食物精灵 → `front`（手）。
  原版 `FoodAnimation.cs` 的注释写得很直白：「第二层夹心为运行时提供」。
  `info.lps` 里 `FoodAnimation#eat:|a0#175,205,23,60,0,0.375:|…` 的 `aN` 就是食物精灵的轨迹：
  `时长,x,y,宽,旋转,不透明度`，只给一个值表示这段时间不显示。
  精灵按原版放进一个 `宽×宽` 的方盒里等比内接（`Height = Width`），绕中心旋转。
  **手在食物前面**，所以看起来是捧着吃——层序错了就穿帮。
  **前后两层帧数不同但总时长相同**（如 Eat/Nomal：后层 19 帧、前层 8 帧，都是 2625 ms），
  所以播放器用「一个时钟 + 每层各自的累计时间表反查帧号」，而不是每层一个游标——后者会漂移。
- **Body 不决定播什么**。它订阅 `pet:state`，用一张纯数据的映射表：

```ts
const CLIP_FOR: Record<Activity, { type: GraphType; name?: string }> = {
  idle:      { type: 'default' },              // + 随机插入 idel/state 类小动作
  working:   { type: 'work', name: 'study' },  // 番茄钟：用 study/studytwo；写代码：workone
  studying:  { type: 'work', name: 'study' },
  break:     { type: 'idel' },
  sleeping:  { type: 'sleep' },
  playing:   { type: 'work', name: 'playone' },
}
// 对话：Brain 开始请求 → think；首个 token 到达 → say；结束 → 回 activity 对应 clip
```

## 4. 窗口与交互

```jsonc
// tauri.conf.json › windows[pet]
{ "label": "pet", "transparent": true, "decorations": false, "alwaysOnTop": true,
  "skipTaskbar": true, "shadow": false, "resizable": false, "width": 500, "height": 500 }
```

- **穿透**：判定不走 touchhead/touchbody 那几个矩形（它们只覆盖头和身体，而 touchraised 是整幅 500 宽的一条带，拿来当命中区太粗），而是用**当前帧的 alpha**：
  - Body 每画一帧，把 500×500 的帧缩到 **48×48**（≈10 逻辑像素/格）读回 alpha，按位打包成 288 字节，掩码变化时才 `set_hit_mask` 推给 Core。缩完再 readback 只有 ~9KB，比直接对原图 `getImageData`（1 MB）便宜两个数量级。
  - Core 每 50 ms 读一次光标，换算成窗口内逻辑坐标查表，**只在结果变化时**才 `set_ignore_cursor_events`。
  - **失败安全**：掩码还没到、光标/窗口位置读不到、状态锁被污染——一律退到「不穿透」。穿透错了宠物就再也点不着（只能从托盘退出），不穿透错了无非挡住下面一次点击，两种代价不对称。
  - 交互期间（按下 / 提起）由 Body 调 `set_hit_test_pinned(true)` 钉住不穿透，否则把宠物拖到光标不再压着它的位置时，轮询会当场把拖拽切断。
  - 托盘留了「鼠标穿透」开关，就是 07 风险表里那条退路的运行时版本。
- **触摸**：摸头 → `touch_head` 三段式；摸身体 → `touch_body`；按住拖动 → `raise`（提起动态）+ 窗口跟随；放下 → 落地。
- **气泡**：同一窗口内的 DOM 层（不另开窗口），贴着窗口底部向上生长、盖在宠物身上——与原版 `MessageBar.xaml`（500×500 的层 + `VerticalAlignment=Bottom`）一致。流式文本、最多 3 行，超出折叠为"展开"。
  > `vup.lps` 里**没有** `say` 锚点（顶层只有 pet/tag/touchhead/touchbody/touchraised/pinch/raisepoint/work/move/duration/bday/side），所以位置不走配置，按原版的底对齐规则来。
  > 折叠不能用 `-webkit-line-clamp`：它在布局层就截断内容，`scrollHeight` 会等于 `clientHeight`，测不出溢出。用 `max-height` 裁。
- **输入**：全局快捷键（默认 `Alt+V` `🔶待确认`，在 Rust 侧注册，不占 JS 的 ACL）或双击宠物 → 气泡下方出现输入框；Esc 先收输入框、再收气泡。
- **托盘**：显示/隐藏、打开 Panel、退出。
- **Panel 窗口**：普通窗口，React 路由 `/memory` `/permissions` `/audit` `/sessions`。

## 5. 身体数值（简化版）

保留原版的体力/心情，**也保留饱腹/口渴**（只去掉金钱）：

```
strength（体力，0–100）：专注时每分钟 −0.3；休息/空闲每分钟 +0.5；睡眠 +1.0
feeling （心情，0–100）：完成番茄钟 +5；完成任务 +8；被摸头 +1（每小时上限）；
                         长时间（>3h）无任何互动 −2/h（只到 40，不再往下——不做惩罚）
hunger  （饱腹，0–100）：随时间下降；低于阈值 → activity 切到 eating，播夹心动画，播完回补
thirst  （口渴，0–100）：同上 → drinking
mood 由体力/心情决定：feeling ≥ 70 → Happy；≥ 40 → Nomal；< 40 或 strength < 20 → PoorCondition；Ill 仅当连续 3 天 PoorCondition
```

饱腹/口渴是**自发行为的燃料**：它给了宠物一个不依赖用户输入、自己会动起来的理由，
这正是「有身体」区别于「聊天框」的地方。数值下降与阈值触发都是确定性代码，放 Rust
`core/state_machine.rs`（Phase 2）；Body 只负责把 `activity` 渲染出来。

数值公式放 Rust `core/state_machine.rs`，可被 `set_setting` 工具调整。原版公式在 `legacy/VPet-Simulator.Core/Display/MainLogic.cs`，只作参考。

## 6. 授权说明

原项目代码 Apache-2.0。`vup` 角色美术资产（萝莉斯）归 LorisYounger / 虚拟主播模拟器，**个人使用无问题，公开分发安装包前需确认美术授权**。方案里 `assets-src/` 进 git（私有仓库可以），构建产物 `public/pet/` 不进 git。
