# 09 · 记忆与检索

> 这份文档写的是 `apps/desktop/src-tauri/src/core/` 下 `memory.rs` / `embed.rs` / `db.rs`
> 三个文件组成的子系统：她**记住什么**、怎么**找回来**、以及为什么是这样。
>
> 04 是设计意图（Phase 4 的原始规划），这份是**已经落地的实现**。两者冲突时以本文为准。

---

## 0. 一句话

存的不是聊天记录，是**可复用的结论**；找的时候不只看语义，
而是「语义 + 重要性 + 新鲜度 + 使用频次 + 置信度」五项加权。

---

## 1. 为什么不是「把对话存下来」

聊天记录是**流水**，记忆是**索引**。两者的生命周期完全相反：

| | 流水 | 索引 |
|---|---|---|
| 增长 | 单调递增 | 应当收敛 |
| 陈旧条目 | 留着无害 | 留着**有害**（稀释检索） |
| 纠正 | 追加一条新的 | 必须**改掉旧的** |
| 删除 | 通常不删 | 用户随时要能删 |

把流水当索引用，结果是检索质量随使用时间单调下降——用得越久越难用。
所以写入的是摘要（「用户正在准备 2027 考研」），不是原文。

---

## 2. 数据模型

### 2.1 表结构（迁移 v3）

```sql
CREATE TABLE memory_items (
    id                TEXT    PRIMARY KEY,
    content           TEXT    NOT NULL,   -- 简洁摘要，不是对话原文
    type              TEXT    NOT NULL,
    importance        REAL    NOT NULL DEFAULT 50,    -- 0–100
    confidence        REAL    NOT NULL DEFAULT 1.0,   -- 0–1
    source            TEXT    NOT NULL,
    pinned            INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    last_accessed_at  INTEGER NOT NULL DEFAULT 0,
    access_count      INTEGER NOT NULL DEFAULT 0,
    expires_at        INTEGER,
    status            TEXT    NOT NULL DEFAULT 'active',
    embedding_version TEXT    NOT NULL DEFAULT '',
    parent_id         TEXT
);
CREATE INDEX idx_memory_status  ON memory_items(status, updated_at DESC);
CREATE INDEX idx_memory_expires ON memory_items(expires_at) WHERE expires_at IS NOT NULL;
```

**每个字段摊成列，不像 `kv` 那样塞 JSON。** 理由是操作决定结构：
要按 `status` 过滤、按 `expires_at` 扫、按 `importance` 排、逐条删——
这些都得让 SQLite 自己能做。塞 JSON 就得全读出来在内存里过一遍。

### 2.2 六种类型

| type | 例子 | 默认有效期 |
|---|---|---|
| `profile` | 用户正在准备 2027 考研 | 无 |
| `preference` | 学习时不喜欢被打扰 | 无 |
| `habit` | 晚上上班，白天学习 | 无 |
| `temporary_context` | 本周在赶项目 | **7 天** |
| `relationship` | 送过礼物，好感上升 | 无 |
| `commitment` | 明天提醒复习行列式 | **30 天** |

临时类**自带**有效期不是可选项：没有它，「本周在赶项目」会永远有效，
半年后还在影响回答。

### 2.3 来源即优先级

```rust
pub enum Source {
    Inferred,       // 未确认的推断 —— 最低
    SystemEvent,
    UserConfirmed,
    UserExplicit,   // 用户亲口说的 —— 最高
}
```

枚举的**声明顺序就是优先级**，`#[derive(PartialOrd, Ord)]` 让 `>` 直接可用。
冲突消解比较的是 `(source, updated_at)` 这个字典序元组，
所以「用户最新明确 > 用户旧明确 > 确认过的推断 > 未确认推断」是一行代码，
不是一串 if。

---

## 3. 写入：三条硬规则

### 3.1 推断不能当事实 —— 在**类型上**堵死

```rust
pub fn plan_write(...) -> WriteOutcome {
    if candidate.source == Source::Inferred {
        return WriteOutcome::NeedsConfirm { item: candidate, why: ConfirmReason::Inferred };
    }
    ...
}
```

`WriteOutcome` 有三个变体，只有 `Created` 和 `Updated` 携带可落库的东西。
**调用方根本拿不到一个能直接写进去的 `MemoryItem`。**
这不是靠调用方自觉，是签名不给机会。

敏感内容同理：关键词表 + 连续 6 位以上数字（卡号 / 证件号 / 手机号大多长这样）。
这是**保守的**检测——漏判会有，误判只是多问一句，代价不对称。

### 3.2 纠正不是新增

写入时在**同类**里找最像的一条，相似度 ≥ 0.55 就判定「在讲同一件事」：

```rust
item.id         = old.id;          // 同一条记忆的新版本
item.created_at = old.created_at;  // 创建时间不该被改写
item.access_count = old.access_count;
item.pinned     = old.pinned;
item.updated_at = now;
```

保住 id 和统计量是关键。否则每纠正一次，使用频次就清零一次，
而使用频次占排序权重的 0.10。

### 3.3 普通闲聊不写

`parse_command` 解析不出记忆命令就返回 `None`，前端据此走正常对话。
这是「不污染记忆库」这条原则**唯一的执行点**——没有别的地方会偷偷写入。

---

## 4. 检索：为什么不能只看相似度

### 4.1 打分公式

```
score = 0.55 · 语义相似度
      + 0.20 · 重要性 / 100
      + 0.10 · 新鲜度
      + 0.10 · 使用频次
      + 0.05 · 置信度
```

各项都归一到 `[0,1]` 再加权：

- **新鲜度**：`0.5^(距今天数 / 30)`，置顶的恒为 `1.0`
- **使用频次**：`n / (n + 5)` —— 饱和映射，用过 5 次算「常用」，
  避免一条被反复命中的记忆无限膨胀
- **重要性 / 置信度**：直接线性归一

**为什么语义只占 0.55**：只看相似度的后果是，三年前随口一句话
因为用词碰巧对上，就盖过今天刚确认的重要信息。语义决定「相不相关」，
其余四项决定「值不值得占用那 5 个名额」。

### 4.2 只取前 5 条

上下文预算是有限的，而且**噪音的代价是超线性的**：多塞两条不相关的记忆，
模型不只是忽略它们，还会试图把它们和问题联系起来。

### 4.3 冲突消解不靠模型

没有 LLM 怎么判断两条记忆矛盾？答案是 **不判断**：

> 同一类型 + 相似度 ≥ 0.55 ⟹ 在说同一件事 ⟹ 只留 `(source, updated_at)` 最大的那条

这会误伤「同一主题的两条互补信息」。这个代价是明知的：
给模型少一条，好过给它两条互相打架的。

### 4.4 生命周期即过滤器

```rust
pub fn usable(&self, now: i64) -> bool {
    self.status == Status::Active && !self.expired(now) && self.source.is_established()
}
```

这是**唯一的裁决点**。删除的、归档的、过期的、未确认的推断，
全都在这一个函数里被挡住。任何检索路径（含向量最近邻）都必须过它——
所以「删掉的还能被召回」这类 bug 不可能出现在多个地方。

---

## 5. 语义向量

### 5.1 字面法的天花板是**结构性**的

两种方法的相似度都能写成同一个二次型：

$$\text{sim}(A,B) = \frac{x_A^\top M\, x_B}{\sqrt{x_A^\top M x_A}\sqrt{x_B^\top M x_B}}$$

区别只在 $M$：

| | $M$ |
|---|---|
| 字符 n-gram | $M = I$ —— 每个符号一根独立的轴，**两两正交** |
| TF-IDF / BM25 | $M = \mathrm{diag}(w)$ —— 仍然正交，只是加权 |
| embedding | $M = W^\top W$ —— $W$ 是学出来的低秩投影 |

$M = I$ 意味着 $\langle\text{作息},\ \text{晚上上班}\rangle = 0$。
零共字 ⟹ 余弦恒为 0。**这不是精度问题，是结构问题**，换再好的加权也解决不了。

embedding 换掉的正是这个 $M$：非对角元素就是符号之间的语义相关性，
而它来自分布假设——上下文分布相似的符号，向量就靠近。

低秩约束（$\mathrm{rank}(M) \le d \ll V$）是**目的不是副作用**：
模型没有足够的维度让每个符号独占一根轴，只能让共现相似的挤到一起。
压缩迫使泛化。

### 5.2 为什么中文字符 n-gram 仍然值得留着

中文的**语素 ≈ 字**：「累」本身就是完整语义单元，「考研」这个 bigram
就是一个完整概念。英文的 `tired` → `ti, ir, re, ed` 是纯拼写噪音。

所以中文的 char-unigram+bigram 实际接近**词级特征**，
对错别字、语序变化、专名精确匹配反而比稠密向量更稳。
两者的死角互补，所以是**混合**，不是替换。

### 5.3 为什么不用 `fastembed`

| | fastembed | `ort` + `tokenizers` |
|---|---|---|
| 依赖 | hf-hub + TLS 栈 + 图像模型 | 两个 crate |
| 模型来源 | 强依赖 HuggingFace 可达 | 本地目录，完全离线 |
| 换模型 | 受支持列表限制 | 任意 ONNX 句向量模型 |
| 池化 / 前缀 | 内置 | 自己控制（见下） |

代价是 mean-pooling、mask 加权、L2 归一化要自己写对。
这几处都有测试盯着（§7.2）。

### 5.4 模型目录

```
<app_data>/embed-model/          # 或 $VPET_EMBED_MODEL
├── model.onnx
├── tokenizer.json
└── embed.json                   # 可选
```

`embed.json`：

```json
{
  "name": "bge-small-zh-v1.5",
  "dim": 512,
  "maxLen": 512,
  "pooling": "cls",
  "queryPrefix": "为这个句子生成表示以用于检索相关文章："
}
```

- **`pooling`** 必须和模型的训练方式一致。bge 系列官方用 **CLS**，
  m3e / text2vec 用 **mean**。选错不会报错，只会悄悄掉点。
- **`queryPrefix`** 只加在**查询**侧，文档侧不加。这不是可有可无的调味——
  bge 就是这么训练的，不加会掉点。
- **`dim` 会被实测值覆盖**。加载时用一句话跑一次推理问出真实维度，
  因为向量表的宽度一旦建错，后面每次写入都会失败。

### 5.5 一切都是可选的

```rust
pub enum EmbedState {
    Disabled { reason: String },   // 没配模型
    Loading,                        // 后台加载中
    Ready { name: String, dim: usize },
    Failed { reason: String },      // 起不来，带原因
}
```

目录不存在、文件损坏、ORT 装不上——任何一种都退回字面检索。
对桌宠来说，「没有语义检索」是降级，「起不来」是故障。

加载在**后台线程**：几十 MB 的模型冷启动要几百毫秒到几秒，
不能挡着宠物出现在桌面上。那段时间字面检索照常工作。

面板上会显示当前用的是哪种。**悄悄降级是最坏的一种降级。**

---

## 6. 向量索引（sqlite-vec）

### 6.1 它不是真相来源

```sql
CREATE VIRTUAL TABLE vec_memories USING vec0(
    memory_id TEXT PRIMARY KEY,
    embedding float[512] distance_metric=cosine
);
```

这张表**不进 `MIGRATIONS`**。迁移是只增不改的定长 SQL，
而这张表的宽度取决于模型维度——换模型就得重建。
用「按需建表 + 维度写在 `kv` 里」比硬塞进迁移序列诚实得多。

删掉它、换个模型重建，记忆一条不少。

### 6.2 换模型 = 整张重建

```rust
pub fn ensure_vec_table(&self, dim: usize, version: &str) -> Result<bool>
```

`kv["vec_meta"]` 存 `"模型名/维度"`。对不上就：

1. `DROP TABLE vec_memories` 并重建
2. `UPDATE memory_items SET embedding_version = ''` —— 让后台去补算

**不同模型的向量在同一个空间里比较是没有意义的，留着比删了更危险。**

### 6.3 余弦的两端要对齐

- `Embedder` 输出 **L2 归一化**向量
- 向量表声明 `distance_metric=cosine`，vec0 返回 `distance = 1 − cos`
- 读回来时 `cosine = 1.0 - distance`

三处必须一致，有集成测试盯着（§7.3）。

### 6.4 为什么现在才引，以及为什么还不算晚

条目几百条时，全表余弦是微秒级——sqlite-vec 带来的加速是 0。
引它的真实理由是**别让"以后要换"变成"以后要重写"**：
写入路径、版本管理、删除同步这几处的正确性和条目数无关，
早一天写对，晚一天就不用在几千条数据上补。

---

## 7. 混合检索

### 7.1 融合

```rust
const DENSE_W: f32 = 0.7;
sim = 0.7 · dense + 0.3 · lexical      // dense 可用时
sim = lexical                           // 否则
```

**为什么是加权和而不是 RRF**：RRF 的长处是融合量纲不同的分数
（BM25 无界，余弦有界）。这里两路都是余弦，直接加权更简单、更可解释，
而且保住了 `[0,1]` 的语义——`MIN_SIMILARITY` 和 `CONFLICT_SIM`
这两个绝对阈值都依赖它。

### 7.2 候选入选的三条通道

```rust
if lexical < MIN_SIMILARITY && dense.is_none() && !m.pinned {
    return None;  // 淘汰
}
```

1. 字面够像（≥ 0.08）
2. **进了向量最近邻**（top 24）
3. 用户置顶（明说了「一直记着」）

第 2 条是关键：**「在不在 KNN 结果里」本身就是稠密侧的门槛**，
不给融合分设绝对阈值。因为真实句向量模型有**各向异性**——
不相干的两句话余弦也常在 0.6 上下，用绝对阈值会把所有东西都放进来。
**相对排名才是稠密检索唯一可信的信号。**

KNN 取 24 而不是 5：最终排序还要过重要性、新鲜度、冲突消解，
只召回 5 个再排序，等于让语义单方面决定了结果。

### 7.3 三个分数都留着

```rust
pub struct Hit {
    pub similarity: f32,      // 融合值，进打分公式
    pub lexical: f32,         // 字面
    pub dense: Option<f32>,   // 语义（None = 不在 KNN 里）
    pub score: f32,
}
```

面板上逐条显示 `总分 ← 字面 · 语义`。
**融合的黑盒不给人看就成了玄学**，调不动。

---

## 8. 一致性：三处跨模块的接缝

单元测试能证明每一段自己是对的，但下面三处的错误**每一边单独看都正确**：

| 接缝 | 两端的约定 | 错了会怎样 |
|---|---|---|
| 归一化 | Embedder 出 L2 归一向量 ↔ 向量表 `distance_metric=cosine` | 相似度系统性偏移，没人会发现 |
| 维度 | 模型实测维度 ↔ 向量表宽度 | 写入直接失败（这个反而好查） |
| 生命周期 | 删除记忆 ↔ 删除向量 | **删了还能被召回** —— 最严重 |
| 内容更新 | 改内容 ↔ 重算向量 | 按**上一版的意思**检索，比找不到更糟 |

`core/pipeline_test.rs` 专门盯这四条，用真 ONNX + 真 sqlite-vec 跑。

---

## 9. 给模型的上下文

不拼数据库原始内容，用固定格式：

```
[用户长期记忆，仅在与当前问题相关时使用]
- [高置信度][长期] 用户正在准备 2027 考研。
- [高置信度][偏好] 用户学习时不喜欢频繁被打扰。
- [临时][有效至 2026-09-20] 用户本周工作压力较大。

规则：
- 仅在确实相关时参考这些记忆。
- 不要捏造未记录的个人信息。
- 不要把推断性记忆说成确定事实。
- 用户纠正记忆时，以用户最新说法为准。
- 不要主动暴露或复述敏感记忆，除非用户当前问题需要。
```

置信度档位：`≥0.8` 高 / `≥0.5` 中 / 其余标注**低置信度·未确认**。

**规则块必须和记忆在同一段里。** 把「不要把推断说成确定事实」放在
system prompt 顶部、记忆放在底部，模型没有理由把这条规则和那条具体记忆联系起来。

没有相关记忆时**返回空串**——没记忆也塞一段提示词是在浪费 token。

---

## 10. 自然语言命令

| 说法 | 动作 |
|---|---|
| 记住：… / 记一下… / 帮我记住… | 写入，重要性 80，`user_explicit` |
| 忘记… / 忘掉… / 别记… | 标记 `deleted` **并删向量** |
| 你记得我什么？ | 按 置顶 → 重要性 列出前 8 条 |
| 设为重要 / 这条很重要 | `pinned = true` |
| 不要再根据…回答 | 标记 `archived`（可恢复，不是删） |

「归档」和「删除」分开是有意的：用户说「别按这条回答」通常不等于
「这件事没发生过」。想彻底删会说「忘记」。

解析在 **Core** 里，走规则表，**不消耗模型调用**——
这些命令是确定性的，没有任何理由让 LLM 来猜。

这也是 roadmap 2.9 那套三层意图识别的第一层。

---

## 11. 生命周期与打扫

跟着心跳走（每 60 秒一次，和状态落库同一个节拍），**只归档不删除**：

```rust
pub fn sweep(items: &[MemoryItem], now: i64) -> Vec<(String, Sweep)>
```

- 过期的（`expires_at <= now`）→ 归档
- 90 天没用过 **且** 重要性 < 40 → 归档
- **置顶的、用户亲口说的 —— 永不自动淘汰**

删除永远是用户的动作。软删除保留行作为审计，
但 `usable()` 把它挡在所有检索之外。

### 健康度

```rust
pub struct Health {
    active, archived, deleted, expired: usize,
    used_ratio: f32,      // 活跃条目里被用过的比例
    avg_importance: f32,
    needs_sweep: usize,
}
```

`used_ratio` 低说明检索被一堆没人看的旧条目稀释了。

---

## 12. 构建

### Cargo features

```toml
default       = ["embed"]
embed         = ["_onnx", "ort/download-binaries"]   # 编译期下载 ORT，静态链接
embed-dynamic = ["_onnx", "ort/load-dynamic"]        # 运行期从 ORT_DYLIB_PATH 加载
_onnx         = ["dep:ort", "dep:tokenizers"]
```

> ⚠️ **`_onnx` 不是多余的。** Cargo 里写了 `dep:ort` 就会**抑制**同名隐式 feature，
> 于是 `#[cfg(feature = "ort")]` 永远为假，整个 ONNX 模块被**静默**编掉且不报错。
> 这个坑踩过一次，`embed.rs` 里留了一条专门的测试守着。

发布版走 `embed`（用户不用额外装东西）。
CI / 离线环境走 `embed-dynamic`：

```bash
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so
cargo test --no-default-features --features embed-dynamic
```

不开 embed 也必须能编过 —— 降级是**结构上的**，不是运行时到处打补丁：

```bash
cargo test --no-default-features      # 203 tests
```

### 装模型

1. 下载一个 ONNX 句向量模型（推荐 `BAAI/bge-small-zh-v1.5`，int8 约 24 MB）
2. 放进 `<app_data>/embed-model/`，需要 `model.onnx` + `tokenizer.json`
3. 写 `embed.json`（bge 系列要 `"pooling": "cls"` 和 `queryPrefix`）
4. 重启；面板「她记得什么」会显示 `语义 + 字面（模型名 · 维度）`
5. 已有记忆会在后台自动补算向量

换模型直接换目录内容重启即可 —— 向量表会自动重建并补算。

---

## 13. 已知边界

- **各向异性未做校正。** 真实句向量的余弦基线偏高（不相干也常在 0.6），
  现在靠「相对排名入选」规避。条目多了可以考虑 whitening / 减去均值向量。
- **不支持多语言混排的最优解。** 中文模型对中英混排的代码片段效果一般。
- **分块没有做。** 记忆是短摘要（一两句话），不需要。
  这条对 Phase 4 的 Obsidian 笔记索引**不成立**，那边要另做分块。
- **`memory_health` 只是统计，没有自动动作。** 目前不会因为健康度低就主动清理。
- **合成模型测试不验证语义质量。** 云端没有 HuggingFace，
  集成测试用的是一张随机词向量表 + 把「作/息」两个字的行改成和「晚/上」相同
  （把语义**打成桩**）。它验的是接缝和链路，不是模型效果——
  模型效果要在真模型上测，那是本地的事。

---

## 14. 相关文件

| 文件 | 职责 |
|---|---|
| `core/memory.rs` | 数据模型、打分、冲突消解、成文、命令解析 —— **纯函数，无 IO** |
| `core/embed.rs` | ONNX 会话、分词、池化、归一化 |
| `core/db.rs` | `memory_items` CRUD + `vec_memories` 向量索引 |
| `core/pipeline_test.rs` | 跨模块接缝的集成测试 |
| `lib.rs` | 后台加载、补算、混合检索编排、Tauri 命令 |
| `panel/Panel.tsx` | 「她记得什么」卡片、检索预览、分数拆解 |
