//! 长期互动记忆（docs/07 roadmap 2.11）。
//!
//! 目标不是「把聊天记录存下来」，是**存可复用的结论**。这两件事差得很远：
//! 聊天记录是流水，记忆是索引；流水越存越多、越查越慢，索引要能被淘汰、被纠正、被删掉。
//!
//! 五条约束决定了这里的全部设计：
//!
//! 1. **SQLite 是唯一真相来源**。向量索引只是加速器，可以随时重建、随时不要。
//!    所以本文件不碰数据库，只做纯计算——存取在 `db.rs`，排序和成文在这里。
//! 2. **推断不能当事实**。AI 猜出来的偏好只能是低置信度候选，要用户点头才转正。
//!    这条在类型上就守住了：`plan_write` 对 `Source::Inferred` 只会回 `NeedsConfirm`，
//!    调用方拿不到一个可以直接落库的 `MemoryItem`。
//! 3. **删除要能审计，但不能再进上下文**。软删除：行还在（那就是审计），
//!    但所有检索入口都按 `status` 过滤。
//! 4. **敏感信息要用户明确确认**。同上，走 `NeedsConfirm`。
//! 5. **排序不能只看语义相似度**。只看相似度的后果是：一条三年前随口说的话，
//!    因为用词碰巧对上了，就盖过今天刚确认过的重要信息。所以是五项加权。
//!
//! 关于 embedding：`Retriever` 是个 trait。现在的默认实现 `BigramRetriever` 不依赖
//! 任何模型——中文短句的语义相似度，字符 unigram + bigram 的余弦已经能吃掉很大一块，
//! 因为中文的**语素本身就携带语义**（「累」这个字就是休息的信号）。
//! Phase 4 的 `fastembed` 到位后原地换掉，`embedding_version` 不一致的条目重算即可。
//! sqlite-vec 等条目上万再说：几百条做全表余弦是微秒级，为它引一个原生扩展不划算。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/* ============================ 数据模型 ============================ */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    /// 稳定身份与目标：「准备 2027 考研」
    Profile,
    /// 偏好：「学习时不喜欢频繁打扰」
    Preference,
    /// 习惯：「晚上工作，白天学习」
    Habit,
    /// 临时状态：「本周在赶项目」。**应当自动过期**
    TemporaryContext,
    /// 角色互动摘要：「送过礼物，好感上升」
    Relationship,
    /// 约定或计划：「明天提醒复习行列式」。**应当带截止日期**
    Commitment,
}

impl MemoryType {
    /// 给上下文用的短标签（docs 里的 `[偏好]` `[临时]`）
    pub fn label(self) -> &'static str {
        match self {
            MemoryType::Profile => "长期",
            MemoryType::Preference => "偏好",
            MemoryType::Habit => "习惯",
            MemoryType::TemporaryContext => "临时",
            MemoryType::Relationship => "互动",
            MemoryType::Commitment => "约定",
        }
    }

    /// 这一类天生就该有有效期吗
    pub fn is_ephemeral(self) -> bool {
        matches!(self, MemoryType::TemporaryContext | MemoryType::Commitment)
    }

    /// 没写有效期时给个默认的（毫秒）。长期类不设
    pub fn default_ttl_ms(self) -> Option<i64> {
        match self {
            MemoryType::TemporaryContext => Some(7 * DAY_MS),
            MemoryType::Commitment => Some(30 * DAY_MS),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "profile" => MemoryType::Profile,
            "preference" => MemoryType::Preference,
            "habit" => MemoryType::Habit,
            "temporary_context" => MemoryType::TemporaryContext,
            "relationship" => MemoryType::Relationship,
            "commitment" => MemoryType::Commitment,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            MemoryType::Profile => "profile",
            MemoryType::Preference => "preference",
            MemoryType::Habit => "habit",
            MemoryType::TemporaryContext => "temporary_context",
            MemoryType::Relationship => "relationship",
            MemoryType::Commitment => "commitment",
        }
    }
}

/// 这条记忆是怎么来的。**冲突时的优先级就是这个枚举的顺序**
/// （docs §5：用户最新明确 > 用户旧明确 > 用户确认过的推断 > 未确认推断）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// 未确认的推断。优先级最低
    Inferred,
    /// 系统事件写的（送礼、好感变化）
    SystemEvent,
    /// AI 推断、用户点过头
    UserConfirmed,
    /// 用户亲口说的。优先级最高
    UserExplicit,
}

impl Source {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "user_explicit" => Source::UserExplicit,
            "user_confirmed" => Source::UserConfirmed,
            "inferred" => Source::Inferred,
            "system_event" => Source::SystemEvent,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Source::UserExplicit => "user_explicit",
            Source::UserConfirmed => "user_confirmed",
            Source::Inferred => "inferred",
            Source::SystemEvent => "system_event",
        }
    }

    /// 这个来源能直接当事实用吗
    pub fn is_established(self) -> bool {
        matches!(self, Source::UserExplicit | Source::UserConfirmed | Source::SystemEvent)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Active,
    Archived,
    Deleted,
}

impl Status {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "active" => Status::Active,
            "archived" => Status::Archived,
            "deleted" => Status::Deleted,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Archived => "archived",
            Status::Deleted => "deleted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryItem {
    pub id: String,
    /// 简洁、可读的记忆摘要。**不是原始对话**
    pub content: String,
    #[serde(rename = "type")]
    pub kind: MemoryType,
    /// 0–100
    pub importance: f32,
    /// 0–1
    pub confidence: f32,
    pub source: Source,
    /// 置顶的永不自动淘汰
    pub pinned: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_accessed_at: i64,
    pub access_count: u32,
    pub expires_at: Option<i64>,
    pub status: Status,
    /// 算相似度用的是哪一版表示。换了模型就按这个筛出要重算的
    pub embedding_version: String,
    /// 同一主题/同一事件的上一条。纠正旧信息时指回去，留下可追溯的链
    pub parent_id: Option<String>,
}

impl MemoryItem {
    pub fn expired(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|t| t <= now)
    }

    /// 能不能进模型上下文。
    /// 删掉的、归档的、过期的、未确认的推断——都不能
    pub fn usable(&self, now: i64) -> bool {
        self.status == Status::Active && !self.expired(now) && self.source.is_established()
    }

    /// 置信度的中文档位。上下文里要标出来，免得把推断说成事实
    pub fn confidence_label(&self) -> &'static str {
        if self.confidence >= 0.8 {
            "高置信度"
        } else if self.confidence >= 0.5 {
            "中置信度"
        } else {
            "低置信度·未确认"
        }
    }
}

pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/* ============================ 相似度 ============================ */

/// 语义相似度的来源。现在是字符 n-gram，Phase 4 换成本地 embedding 模型，
/// 这个接口不变——调用方（排序、写入查重）一行都不用改
pub trait Retriever: Send + Sync {
    /// 0–1
    fn similarity(&self, query: &str, content: &str) -> f32;
    /// 写进 `embedding_version`。换实现时用它筛出要重算的条目
    fn version(&self) -> &str;
}

/// 「穷人的 embedding」：字符 unigram + bigram 的 TF 余弦。
///
/// 为什么对中文管用：中文的**语素本身携带语义**，「累」这个字就是休息的信号，
/// 「考研」这个 bigram 就是一个完整概念。unigram 兜单字语义，bigram 兜词，
/// 两者加权相加再做余弦，对错别字、语序变化、加语气词都鲁棒。
///
/// 它当然比不过真模型——比不过的是「别太拼了」→「休息」这种没有共同字的泛化。
/// 那部分等 Phase 4。
pub struct BigramRetriever;

const UNIGRAM_W: f32 = 0.5;
const BIGRAM_W: f32 = 1.0;

fn tokens(s: &str) -> HashMap<String, f32> {
    let chars: Vec<char> = s
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect();
    let mut m: HashMap<String, f32> = HashMap::new();
    for c in &chars {
        *m.entry(c.to_string()).or_insert(0.0) += UNIGRAM_W;
    }
    for w in chars.windows(2) {
        *m.entry(w.iter().collect()).or_insert(0.0) += BIGRAM_W;
    }
    m
}

fn cosine(a: &HashMap<String, f32>, b: &HashMap<String, f32>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().map(|(k, v)| b.get(k).map_or(0.0, |w| v * w)).sum();
    let na: f32 = a.values().map(|v| v * v).sum::<f32>().sqrt();
    let nb: f32 = b.values().map(|v| v * v).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        (dot / (na * nb)).clamp(0.0, 1.0)
    }
}

impl Retriever for BigramRetriever {
    fn similarity(&self, query: &str, content: &str) -> f32 {
        cosine(&tokens(query), &tokens(content))
    }
    fn version(&self) -> &str {
        "bigram-v1"
    }
}

/* ============================ 排序 ============================ */

/// docs §5 的加权，权重不动。
/// 分开写成常量是为了能一眼看出「语义只占一半多一点」——
/// 只看相似度的后果是三年前随口一句话因为用词碰巧对上就盖过今天刚确认的重要信息
const W_SEMANTIC: f32 = 0.55;
const W_IMPORTANCE: f32 = 0.20;
const W_RECENCY: f32 = 0.10;
const W_USAGE: f32 = 0.10;
const W_CONFIDENCE: f32 = 0.05;

/// 新鲜度的半衰期：一个月。置顶的恒为 1
const RECENCY_HALF_LIFE_DAYS: f32 = 30.0;
/// 使用频次饱和到 0–1 的拐点：用过 5 次就算「常用」
const USAGE_K: f32 = 5.0;

/// 最终送进模型的条数上限（docs §5：3~6 条）
pub const TOP_K: usize = 5;
/// 相似度低于此的根本不算候选，免得凑数把不相干的塞进去
const MIN_SIMILARITY: f32 = 0.08;

fn recency(item: &MemoryItem, now: i64) -> f32 {
    if item.pinned {
        return 1.0;
    }
    let days = ((now - item.updated_at).max(0) as f32) / DAY_MS as f32;
    0.5f32.powf(days / RECENCY_HALF_LIFE_DAYS)
}

fn usage(item: &MemoryItem) -> f32 {
    let n = item.access_count as f32;
    n / (n + USAGE_K)
}

/// 一条记忆对当前问题的得分。各项都归一到 0–1 再加权
pub fn score(item: &MemoryItem, similarity: f32, now: i64) -> f32 {
    W_SEMANTIC * similarity
        + W_IMPORTANCE * (item.importance / 100.0).clamp(0.0, 1.0)
        + W_RECENCY * recency(item, now)
        + W_USAGE * usage(item)
        + W_CONFIDENCE * item.confidence.clamp(0.0, 1.0)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub item: MemoryItem,
    pub similarity: f32,
    pub score: f32,
}

/// 检索：过滤 → 打分 → 消解冲突 → 取前 k 条。
///
/// 三步的顺序不能换。先过滤（删掉的、过期的、未确认的推断根本不该参与），
/// 再打分，**最后**才消解冲突——冲突消解要在已排序的候选里做，
/// 否则会因为一条低分的矛盾记忆把高分的挤掉。
pub fn retrieve(
    items: &[MemoryItem],
    query: &str,
    retr: &dyn Retriever,
    now: i64,
    k: usize,
) -> Vec<Hit> {
    let mut hits: Vec<Hit> = items
        .iter()
        .filter(|m| m.usable(now))
        .filter_map(|m| {
            let similarity = retr.similarity(query, &m.content);
            // 置顶的记忆不管问什么都该在候选里——那是用户明说了「一直记着」
            if similarity < MIN_SIMILARITY && !m.pinned {
                return None;
            }
            Some(Hit {
                score: score(m, similarity, now),
                similarity,
                item: m.clone(),
            })
        })
        .collect();
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    resolve_conflicts(&mut hits, retr);
    hits.truncate(k);
    hits
}

/// 相似到这个程度、又是同一类，就当成在讲同一件事
const CONFLICT_SIM: f32 = 0.55;

/// 同一主题只留一条。
///
/// 优先级：用户最新明确 > 用户旧明确 > 用户确认过的推断 > 未确认推断
/// ——也就是先比 `source`，同源再比 `updated_at`。
/// 「没有 LLM 怎么判断两条记忆矛盾」的答案是：**不判断**。
/// 同类 + 高相似 = 在说同一件事，那就只留优先级最高的那条。
/// 这会误伤「同一主题的两条互补信息」，代价是少给模型一条，
/// 比给它两条互相打架的强。
fn resolve_conflicts(hits: &mut Vec<Hit>, retr: &dyn Retriever) {
    let mut kept: Vec<Hit> = Vec::with_capacity(hits.len());
    for h in hits.drain(..) {
        let clash = kept.iter_mut().find(|k| {
            k.item.kind == h.item.kind
                && retr.similarity(&k.item.content, &h.item.content) >= CONFLICT_SIM
        });
        match clash {
            Some(k) => {
                let better = (h.item.source, h.item.updated_at) > (k.item.source, k.item.updated_at);
                if better {
                    *k = h;
                }
            }
            None => kept.push(h),
        }
    }
    *hits = kept;
}

/* ============================ 写入 ============================ */

/// 写到这个相似度以上就算「在纠正同一条」，走更新而不是新增
const SUPERSEDE_SIM: f32 = 0.55;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum WriteOutcome {
    /// 新的一条，直接落库
    Created { item: MemoryItem },
    /// 在纠正已有的那条：更新它，不新增一条互相矛盾的
    Updated {
        item: MemoryItem,
        replaced_id: String,
        replaced_content: String,
    },
    /// 不能直接落库，要用户先点头（推断出来的，或者内容敏感）
    NeedsConfirm { item: MemoryItem, why: ConfirmReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmReason {
    /// AI 猜的，不能当事实
    Inferred,
    /// 内容看着敏感，长期保存要用户明说
    Sensitive,
}

impl ConfirmReason {
    pub fn question(self, content: &str) -> String {
        match self {
            ConfirmReason::Inferred => format!("我猜「{content}」，要我记下来吗？"),
            ConfirmReason::Sensitive => {
                format!("「{content}」看着像是不方便留底的信息，确定要我一直记着吗？")
            }
        }
    }
}

/// 看着像敏感信息的词。宁可多问一句，也不要把这种东西默默存一辈子。
/// 这是**保守的**关键词法——漏判会有，误判只是多问一次，代价不对称
const SENSITIVE_WORDS: [&str; 16] = [
    "密码", "口令", "身份证", "银行卡", "信用卡", "验证码", "私钥", "token",
    "住址", "家庭住址", "病历", "诊断", "确诊", "工资", "薪资", "存款",
];

/// 连续 6 位以上数字：卡号、证件号、手机号大多长这样
fn has_long_digit_run(s: &str) -> bool {
    let mut run = 0;
    for c in s.chars() {
        if c.is_ascii_digit() {
            run += 1;
            if run >= 6 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

pub fn looks_sensitive(content: &str) -> bool {
    let lower = content.to_lowercase();
    SENSITIVE_WORDS.iter().any(|w| lower.contains(w)) || has_long_digit_run(content)
}

/// 造一条候选记忆（还没落库）
#[allow(clippy::too_many_arguments)]
pub fn draft(
    id: String,
    content: String,
    kind: MemoryType,
    importance: f32,
    confidence: f32,
    source: Source,
    expires_at: Option<i64>,
    version: &str,
    now: i64,
) -> MemoryItem {
    MemoryItem {
        id,
        content: content.trim().to_string(),
        kind,
        importance: importance.clamp(0.0, 100.0),
        confidence: confidence.clamp(0.0, 1.0),
        source,
        pinned: false,
        created_at: now,
        updated_at: now,
        last_accessed_at: 0,
        access_count: 0,
        // 临时类和约定类没写有效期的，给个默认的——否则「本周在赶项目」会永远有效
        expires_at: expires_at.or_else(|| kind.default_ttl_ms().map(|ttl| now + ttl)),
        status: Status::Active,
        embedding_version: version.to_string(),
        parent_id: None,
    }
}

/// 决定这条候选该怎么落：新增、更新旧的、还是先问一句。
///
/// **这个函数是「推断不能当事实」的执行点**：`Inferred` 永远走 `NeedsConfirm`，
/// 调用方拿不到一个能直接落库的 `MemoryItem`。类型上就堵死了。
pub fn plan_write(
    existing: &[MemoryItem],
    candidate: MemoryItem,
    retr: &dyn Retriever,
    now: i64,
) -> WriteOutcome {
    if candidate.source == Source::Inferred {
        return WriteOutcome::NeedsConfirm {
            item: candidate,
            why: ConfirmReason::Inferred,
        };
    }
    if looks_sensitive(&candidate.content) && candidate.source != Source::UserConfirmed {
        return WriteOutcome::NeedsConfirm {
            item: candidate,
            why: ConfirmReason::Sensitive,
        };
    }

    // 同一类里找最像的那条：找到了就是在纠正它，更新，不新增互相矛盾的
    let near = existing
        .iter()
        .filter(|m| m.status == Status::Active && m.kind == candidate.kind && !m.expired(now))
        .map(|m| (m, retr.similarity(&m.content, &candidate.content)))
        .filter(|(_, s)| *s >= SUPERSEDE_SIM)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    match near {
        Some((old, _)) => {
            let mut item = candidate;
            // 保留身份和统计：这是同一条记忆的新版本，不是新记忆
            item.id = old.id.clone();
            item.created_at = old.created_at;
            item.access_count = old.access_count;
            item.last_accessed_at = old.last_accessed_at;
            item.pinned = old.pinned;
            item.parent_id = old.parent_id.clone();
            item.updated_at = now;
            WriteOutcome::Updated {
                replaced_id: old.id.clone(),
                replaced_content: old.content.clone(),
                item,
            }
        }
        None => WriteOutcome::Created { item: candidate },
    }
}

/* ============================ 生命周期 ============================ */

/// 长期没用、又不重要的，先归档不删
const STALE_DAYS: f32 = 90.0;
const STALE_IMPORTANCE: f32 = 40.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sweep {
    /// 过期了
    Expire,
    /// 长期没用又不重要
    Archive,
}

/// 例行打扫：算出哪些该失效、哪些该归档。**不删东西**——
/// 归档是可逆的，删除必须是用户的动作
pub fn sweep(items: &[MemoryItem], now: i64) -> Vec<(String, Sweep)> {
    items
        .iter()
        .filter(|m| m.status == Status::Active)
        // 置顶和用户亲口说的永不自动淘汰（docs §4.6）
        .filter(|m| !m.pinned && m.source != Source::UserExplicit)
        .filter_map(|m| {
            if m.expired(now) {
                return Some((m.id.clone(), Sweep::Expire));
            }
            let idle_days = ((now - m.last_accessed_at.max(m.updated_at)).max(0) as f32) / DAY_MS as f32;
            if idle_days >= STALE_DAYS && m.importance < STALE_IMPORTANCE {
                return Some((m.id.clone(), Sweep::Archive));
            }
            None
        })
        .collect()
}

/// 记忆库健康度：活跃条目里有多少是「还在被用的」。
/// 低了说明检索被一堆没人看的旧条目稀释，该清了
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub active: usize,
    pub archived: usize,
    pub deleted: usize,
    pub expired: usize,
    /// 活跃条目里被用过的比例，0–1
    pub used_ratio: f32,
    /// 活跃条目的平均重要性
    pub avg_importance: f32,
    /// 需要打扫的条数
    pub needs_sweep: usize,
}

pub fn health(items: &[MemoryItem], now: i64) -> Health {
    let active: Vec<&MemoryItem> = items.iter().filter(|m| m.status == Status::Active).collect();
    let used = active.iter().filter(|m| m.access_count > 0).count();
    Health {
        active: active.len(),
        archived: items.iter().filter(|m| m.status == Status::Archived).count(),
        deleted: items.iter().filter(|m| m.status == Status::Deleted).count(),
        expired: active.iter().filter(|m| m.expired(now)).count(),
        used_ratio: if active.is_empty() { 1.0 } else { used as f32 / active.len() as f32 },
        avg_importance: if active.is_empty() {
            0.0
        } else {
            active.iter().map(|m| m.importance).sum::<f32>() / active.len() as f32
        },
        needs_sweep: sweep(items, now).len(),
    }
}

/* ============================ 成文 ============================ */

/// 把毫秒时间戳写成 `YYYY-MM-DD`
fn ymd(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "?".into())
}

/// 给模型的上下文（docs §7）。
///
/// **不拼数据库原始内容**：格式是固定的，每条都标出置信度和类型，
/// 临时的还要标有效期。规则块跟在后面——「不要把推断说成确定事实」这句话
/// 必须和那条低置信度记忆出现在同一段里，否则模型没有理由区别对待它们。
pub fn render_context(hits: &[Hit]) -> String {
    if hits.is_empty() {
        return String::new();
    }
    let mut out = String::from("[用户长期记忆，仅在与当前问题相关时使用]\n");
    for h in hits {
        let m = &h.item;
        let mut tags = format!("[{}][{}]", m.confidence_label(), m.kind.label());
        if let Some(exp) = m.expires_at {
            tags.push_str(&format!("[有效至 {}]", ymd(exp)));
        }
        out.push_str(&format!("- {} {}\n", tags, m.content));
    }
    out.push_str(
        "\n规则：\n\
         - 仅在确实相关时参考这些记忆。\n\
         - 不要捏造未记录的个人信息。\n\
         - 不要把推断性记忆说成确定事实。\n\
         - 用户纠正记忆时，以用户最新说法为准。\n\
         - 不要主动暴露或复述敏感记忆，除非用户当前问题需要。\n",
    );
    out
}

/* ============================ 自然语言命令 ============================ */

/// 用户对记忆库能下的指令（docs §8）。
///
/// 这是 roadmap 2.9 那套三层意图识别的**第一层**：规则表。
/// 记忆命令的句式高度固定（「记住：…」「忘记…」「你记得我什么」），
/// 规则层就能吃掉绝大部分，用不着惊动模型。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "cmd")]
pub enum Command {
    /// 记住这件事
    Remember { content: String, kind: MemoryType, importance: f32, ttl_ms: Option<i64> },
    /// 忘掉和这句话相关的
    Forget { query: String },
    /// 你记得我什么
    Recall,
    /// 把最近提到的那条设为重要 / 置顶
    Pin { query: String },
    /// 别再按这条回答了（= 归档，不是删）
    Mute { query: String },
}

const REMEMBER_PREFIX: [&str; 6] = ["记住：", "记住:", "记住", "记一下", "帮我记住", "记下"];
const FORGET_PREFIX: [&str; 5] = ["忘记：", "忘记:", "忘记", "忘掉", "别记"];
const RECALL_PHRASES: [&str; 5] = ["你记得我什么", "你还记得什么", "记得我什么", "你记住了什么", "你记得什么"];
const PIN_PREFIX: [&str; 4] = ["设为重要", "这条很重要", "把这条设为重要信息", "重要："];
const MUTE_PREFIX: [&str; 3] = ["不要再根据", "别再根据", "不要按这条"];

/// 临时性的说法 → 有效期
const TEMPORARY_HINTS: [(&str, i64); 6] = [
    ("到周末", 3 * DAY_MS),
    ("这周", 7 * DAY_MS),
    ("本周", 7 * DAY_MS),
    ("这几天", 3 * DAY_MS),
    ("最近", 14 * DAY_MS),
    ("暂时", 7 * DAY_MS),
];

/// 内容 → 记忆类型的线索词。命中多个取第一个；都不中按 profile
const TYPE_HINTS: [(&str, MemoryType); 14] = [
    ("不喜欢", MemoryType::Preference),
    ("喜欢", MemoryType::Preference),
    ("讨厌", MemoryType::Preference),
    ("偏好", MemoryType::Preference),
    ("习惯", MemoryType::Habit),
    ("每天", MemoryType::Habit),
    ("总是", MemoryType::Habit),
    ("一般都", MemoryType::Habit),
    ("提醒我", MemoryType::Commitment),
    ("答应", MemoryType::Commitment),
    ("约好", MemoryType::Commitment),
    ("计划", MemoryType::Commitment),
    ("准备考", MemoryType::Profile),
    ("目标", MemoryType::Profile),
];

fn strip_prefix<'a>(text: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    for p in prefixes {
        if let Some(rest) = text.strip_prefix(p) {
            return Some(rest.trim_start_matches([':', '：', ' ', '，', ',']).trim());
        }
    }
    None
}

/// 猜这句话该归成哪一类，以及该不该带有效期。
/// 「猜」是诚实的说法——这是关键词法，不是理解
pub fn classify(content: &str) -> (MemoryType, Option<i64>) {
    let ttl = TEMPORARY_HINTS
        .iter()
        .find(|(w, _)| content.contains(w))
        .map(|(_, d)| *d);
    if ttl.is_some() {
        return (MemoryType::TemporaryContext, ttl);
    }
    let kind = TYPE_HINTS
        .iter()
        .find(|(w, _)| content.contains(w))
        .map(|(_, k)| *k)
        .unwrap_or(MemoryType::Profile);
    (kind, kind.default_ttl_ms())
}

/// 把一句中文解析成记忆命令。解析不出来就返回 None——
/// **普通闲聊默认不写长期记忆**（docs §4.3），这是那条原则的执行点
pub fn parse_command(text: &str) -> Option<Command> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if RECALL_PHRASES.iter().any(|p| t.contains(p)) {
        return Some(Command::Recall);
    }
    if let Some(rest) = strip_prefix(t, &MUTE_PREFIX) {
        return Some(Command::Mute { query: rest.to_string() });
    }
    if let Some(rest) = strip_prefix(t, &FORGET_PREFIX) {
        if rest.is_empty() {
            return None;
        }
        return Some(Command::Forget { query: rest.to_string() });
    }
    if let Some(rest) = strip_prefix(t, &PIN_PREFIX) {
        return Some(Command::Pin { query: rest.to_string() });
    }
    if let Some(rest) = strip_prefix(t, &REMEMBER_PREFIX) {
        if rest.is_empty() {
            return None;
        }
        let (kind, ttl_ms) = classify(rest);
        // 用户亲口让记的，重要性给高——这是他主动挑出来的信息
        return Some(Command::Remember {
            content: rest.to_string(),
            kind,
            importance: 80.0,
            ttl_ms,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_760_000_000_000;

    fn r() -> BigramRetriever {
        BigramRetriever
    }

    fn mk(id: &str, content: &str, kind: MemoryType, source: Source) -> MemoryItem {
        draft(id.into(), content.into(), kind, 60.0, 0.9, source, None, "bigram-v1", NOW)
    }

    /* ---------- 相似度 ---------- */

    #[test]
    fn 相似度认得出同一件事的不同说法() {
        let r = r();
        let a = "用户正在准备 2027 考研";
        assert!(r.similarity(a, "我在准备考研") > 0.3, "{}", r.similarity(a, "我在准备考研"));
        assert!(r.similarity(a, "今天天气不错") < 0.15);
    }

    #[test]
    fn 相似度对语序和语气词鲁棒() {
        let r = r();
        let base = "晚上上班白天学习";
        assert!(r.similarity(base, "我晚上上班，白天学习的") > 0.7);
    }

    /// 记下这个实现的天花板：没有共同字的同义表达它够不着。
    /// 「作息」和「晚上上班白天学习」说的是一回事，bigram 认不出来——
    /// 这正是 Phase 4 换本地 embedding 模型要解决的那部分长尾。
    /// 这条测试是**规格说明**，不是缺陷报告：换了模型之后它应该失败，那时改掉它
    #[test]
    fn 已知上限_没有共同字的同义词认不出来() {
        assert!(r().similarity("作息", "用户晚上上班白天学习") < MIN_SIMILARITY);
    }

    #[test]
    fn 空串不会算出诡异的相似度() {
        let r = r();
        assert_eq!(r.similarity("", "任何东西"), 0.0);
        assert_eq!(r.similarity("任何东西", ""), 0.0);
    }

    /* ---------- 排序 ---------- */

    #[test]
    fn 语义相似但不重要的排不过重要的() {
        let mut low = mk("a", "用户在学线性代数", MemoryType::Profile, Source::UserExplicit);
        low.importance = 5.0;
        let mut high = mk("b", "用户在学线性代数复习行列式", MemoryType::Habit, Source::UserExplicit);
        high.importance = 95.0;
        let hits = retrieve(&[low, high], "线性代数", &r(), NOW, TOP_K);
        assert_eq!(hits[0].item.id, "b", "重要性 0.20 的权重没起作用");
    }

    #[test]
    fn 陈旧的记忆分数会衰减() {
        let fresh = mk("new", "用户在准备考研", MemoryType::Profile, Source::UserExplicit);
        let mut old = fresh.clone();
        old.id = "old".into();
        old.updated_at = NOW - 365 * DAY_MS;
        assert!(score(&fresh, 0.5, NOW) > score(&old, 0.5, NOW));
    }

    #[test]
    fn 置顶的记忆不管问什么都在候选里() {
        let mut pinned = mk("p", "用户对花生过敏", MemoryType::Profile, Source::UserExplicit);
        pinned.pinned = true;
        let hits = retrieve(&[pinned], "今天写代码累不累", &r(), NOW, TOP_K);
        assert_eq!(hits.len(), 1, "置顶的被相似度门槛滤掉了");
    }

    #[test]
    fn 未确认的推断不参与回答() {
        let guess = mk("g", "用户大概喜欢喝咖啡", MemoryType::Preference, Source::Inferred);
        let hits = retrieve(&[guess], "咖啡", &r(), NOW, TOP_K);
        assert!(hits.is_empty(), "未确认的推断进了上下文");
    }

    #[test]
    fn 删掉和归档的都不参与回答() {
        let mut gone = mk("d", "用户晚上上班", MemoryType::Habit, Source::UserExplicit);
        gone.status = Status::Deleted;
        let mut filed = mk("a", "用户晚上上班", MemoryType::Habit, Source::UserExplicit);
        filed.status = Status::Archived;
        assert!(retrieve(&[gone, filed], "晚上上班", &r(), NOW, TOP_K).is_empty());
    }

    #[test]
    fn 最多只取_k_条() {
        let items: Vec<_> = (0..20)
            .map(|i| mk(&format!("m{i}"), &format!("用户在学科目{i}"), MemoryType::Profile, Source::UserExplicit))
            .collect();
        assert!(retrieve(&items, "用户在学什么", &r(), NOW, TOP_K).len() <= TOP_K);
    }

    /* ---------- 冲突消解 ---------- */

    #[test]
    fn 冲突时用户最新的明确说法胜出() {
        let mut old = mk("old", "用户晚上上班白天学习", MemoryType::Habit, Source::UserExplicit);
        old.updated_at = NOW - 10 * DAY_MS;
        let new = mk("new", "用户晚上学习白天上班", MemoryType::Habit, Source::UserExplicit);
        let hits = retrieve(&[old, new], "我晚上干什么", &r(), NOW, TOP_K);
        assert_eq!(hits.len(), 1, "两条矛盾的都送进去了");
        assert_eq!(hits[0].item.id, "new");
    }

    #[test]
    fn 明确说法压过确认过的推断() {
        let confirmed = mk("c", "用户喜欢安静的学习环境", MemoryType::Preference, Source::UserConfirmed);
        let mut explicit = mk("e", "用户喜欢安静的学习环境不被打扰", MemoryType::Preference, Source::UserExplicit);
        explicit.updated_at = NOW - 30 * DAY_MS; // 更旧，但来源更硬
        let hits = retrieve(&[confirmed, explicit], "学习环境", &r(), NOW, TOP_K);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item.id, "e", "来源优先级没压过时间");
    }

    #[test]
    fn 不同类型的相似记忆不算冲突() {
        let a = mk("a", "用户在准备考研", MemoryType::Profile, Source::UserExplicit);
        let b = mk("b", "用户在准备考研", MemoryType::Commitment, Source::UserExplicit);
        assert_eq!(retrieve(&[a, b], "考研", &r(), NOW, TOP_K).len(), 2);
    }

    /* ---------- 写入 ---------- */

    #[test]
    fn 推断只能是候选不能直接落库() {
        let guess = mk("g", "用户可能喜欢咖啡", MemoryType::Preference, Source::Inferred);
        match plan_write(&[], guess, &r(), NOW) {
            WriteOutcome::NeedsConfirm { why, .. } => assert_eq!(why, ConfirmReason::Inferred),
            other => panic!("推断被直接写进去了：{other:?}"),
        }
    }

    #[test]
    fn 敏感信息要先问一句() {
        let secret = mk("s", "用户的银行卡密码是 123456", MemoryType::Profile, Source::UserExplicit);
        match plan_write(&[], secret, &r(), NOW) {
            WriteOutcome::NeedsConfirm { why, .. } => assert_eq!(why, ConfirmReason::Sensitive),
            other => panic!("敏感信息被默默存了：{other:?}"),
        }
    }

    #[test]
    fn 用户确认过的敏感信息可以存() {
        let secret = mk("s", "用户的住址在城西", MemoryType::Profile, Source::UserConfirmed);
        assert!(matches!(plan_write(&[], secret, &r(), NOW), WriteOutcome::Created { .. }));
    }

    #[test]
    fn 长串数字也算敏感() {
        assert!(looks_sensitive("手机号 13800138000"));
        assert!(!looks_sensitive("用户在准备 2027 考研"));
    }

    #[test]
    fn 纠正旧信息是更新不是新增() {
        let old = mk("old", "用户晚上上班白天学习", MemoryType::Habit, Source::UserExplicit);
        let new = mk("new", "用户晚上上班白天学习的", MemoryType::Habit, Source::UserExplicit);
        match plan_write(std::slice::from_ref(&old), new, &r(), NOW) {
            WriteOutcome::Updated { item, replaced_id, .. } => {
                assert_eq!(replaced_id, "old");
                assert_eq!(item.id, "old", "更新应当保住原来的身份");
                assert_eq!(item.created_at, old.created_at, "创建时间不该被改写");
            }
            other => panic!("新增了一条互相矛盾的：{other:?}"),
        }
    }

    #[test]
    fn 不相干的内容是新增() {
        let old = mk("old", "用户在准备考研", MemoryType::Profile, Source::UserExplicit);
        let new = mk("new", "用户养了一只橘猫", MemoryType::Profile, Source::UserExplicit);
        assert!(matches!(plan_write(&[old], new, &r(), NOW), WriteOutcome::Created { .. }));
    }

    #[test]
    fn 临时记忆自带有效期() {
        let m = draft("t".into(), "本周在赶项目".into(), MemoryType::TemporaryContext,
                      60.0, 0.9, Source::UserExplicit, None, "bigram-v1", NOW);
        assert!(m.expires_at.is_some(), "临时状态没有有效期，会永远有效");
        assert!(!m.expired(NOW));
        assert!(m.expired(NOW + 8 * DAY_MS));
    }

    #[test]
    fn 长期记忆不自带有效期() {
        let m = mk("p", "用户在准备考研", MemoryType::Profile, Source::UserExplicit);
        assert!(m.expires_at.is_none());
    }

    /* ---------- 生命周期 ---------- */

    #[test]
    fn 过期的记忆不再参与回答() {
        let mut m = mk("t", "用户本周工作压力大", MemoryType::TemporaryContext, Source::UserExplicit);
        m.expires_at = Some(NOW - 1);
        assert!(retrieve(&[m], "工作压力", &r(), NOW, TOP_K).is_empty());
    }

    #[test]
    fn 打扫只归档不删除() {
        let mut stale = mk("s", "用户提过一次某个工具", MemoryType::Preference, Source::SystemEvent);
        stale.importance = 10.0;
        stale.updated_at = NOW - 200 * DAY_MS;
        stale.last_accessed_at = NOW - 200 * DAY_MS;
        let todo = sweep(&[stale], NOW);
        assert_eq!(todo.len(), 1);
        assert_eq!(todo[0].1, Sweep::Archive);
    }

    #[test]
    fn 置顶和用户明说的永不自动淘汰() {
        let mut pinned = mk("p", "随便什么", MemoryType::Preference, Source::SystemEvent);
        pinned.pinned = true;
        pinned.importance = 0.0;
        pinned.updated_at = NOW - 999 * DAY_MS;
        pinned.expires_at = Some(NOW - 1);

        let mut explicit = mk("e", "用户亲口说的", MemoryType::Preference, Source::UserExplicit);
        explicit.importance = 0.0;
        explicit.updated_at = NOW - 999 * DAY_MS;

        assert!(sweep(&[pinned, explicit], NOW).is_empty(), "不该自动淘汰的被扫了");
    }

    #[test]
    fn 健康度算得出来() {
        let mut used = mk("a", "甲", MemoryType::Profile, Source::UserExplicit);
        used.access_count = 3;
        let unused = mk("b", "乙", MemoryType::Profile, Source::UserExplicit);
        let h = health(&[used, unused], NOW);
        assert_eq!(h.active, 2);
        assert!((h.used_ratio - 0.5).abs() < 1e-6);
    }

    /* ---------- 成文 ---------- */

    #[test]
    fn 上下文按固定格式成文() {
        let mut a = mk("a", "用户正在准备 2027 考研", MemoryType::Profile, Source::UserExplicit);
        a.confidence = 0.95;
        let mut b = mk("b", "用户本周工作压力较大", MemoryType::TemporaryContext, Source::UserExplicit);
        b.confidence = 0.9;
        b.expires_at = Some(NOW + 3 * DAY_MS);
        let hits = retrieve(&[a, b], "考研 压力", &r(), NOW, TOP_K);
        let text = render_context(&hits);
        assert!(text.starts_with("[用户长期记忆，仅在与当前问题相关时使用]"));
        assert!(text.contains("[高置信度][长期] 用户正在准备 2027 考研"));
        assert!(text.contains("[临时][有效至 "), "临时记忆没标有效期：\n{text}");
        assert!(text.contains("不要把推断性记忆说成确定事实"));
    }

    #[test]
    fn 没有相关记忆时上下文是空的() {
        assert_eq!(render_context(&[]), "", "没记忆也塞一段提示词是在浪费 token");
    }

    /* ---------- 自然语言命令 ---------- */

    #[test]
    fn 认得出记住和忘记() {
        match parse_command("记住：我晚上上班，白天学习") {
            Some(Command::Remember { content, .. }) => assert_eq!(content, "我晚上上班，白天学习"),
            other => panic!("{other:?}"),
        }
        match parse_command("忘记我晚上上班") {
            Some(Command::Forget { query }) => assert_eq!(query, "我晚上上班"),
            other => panic!("{other:?}"),
        }
        assert_eq!(parse_command("你记得我什么？"), Some(Command::Recall));
    }

    #[test]
    fn 普通闲聊不写长期记忆() {
        for chat in ["今天天气真好", "你在干嘛", "哈哈哈", "嗯", "帮我算一下 3 加 5"] {
            assert_eq!(parse_command(chat), None, "「{chat}」被当成记忆命令了");
        }
    }

    #[test]
    fn 暂时性的说法会带上有效期() {
        match parse_command("记住：我这周在赶项目") {
            Some(Command::Remember { kind, ttl_ms, .. }) => {
                assert_eq!(kind, MemoryType::TemporaryContext);
                assert!(ttl_ms.is_some());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn 偏好和习惯能被分出来() {
        assert!(matches!(
            parse_command("记住：我学习时不喜欢被打扰"),
            Some(Command::Remember { kind: MemoryType::Preference, .. })
        ));
        assert!(matches!(
            parse_command("记住：我每天七点起床"),
            Some(Command::Remember { kind: MemoryType::Habit, .. })
        ));
        assert!(matches!(
            parse_command("记住：提醒我复习行列式"),
            Some(Command::Remember { kind: MemoryType::Commitment, .. })
        ));
    }

    #[test]
    fn 空的记住命令不算命令() {
        assert_eq!(parse_command("记住："), None);
        assert_eq!(parse_command("忘记"), None);
    }

    /* ---------- 验收标准（docs §9）逐条 ---------- */

    #[test]
    fn 验收1_记住作息之后问学习安排能引用到() {
        let cmd = parse_command("记住：我晚上上班，白天学习").unwrap();
        let Command::Remember { content, kind, importance, ttl_ms } = cmd else { panic!() };
        let item = draft("m1".into(), content, kind, importance, 1.0,
                         Source::UserExplicit, ttl_ms.map(|t| NOW + t), "bigram-v1", NOW);
        let WriteOutcome::Created { item } = plan_write(&[], item, &r(), NOW) else { panic!() };
        let hits = retrieve(&[item], "我白天该怎么安排学习", &r(), NOW, TOP_K);
        assert_eq!(hits.len(), 1);
        assert!(render_context(&hits).contains("晚上上班"));
    }

    #[test]
    fn 验收2_忘记之后不再进上下文() {
        let mut item = mk("m1", "我晚上上班，白天学习", MemoryType::Habit, Source::UserExplicit);
        assert!(!retrieve(std::slice::from_ref(&item), "学习安排", &r(), NOW, TOP_K).is_empty());
        // 「忘记」= 标记删除
        item.status = Status::Deleted;
        assert!(retrieve(&[item], "学习安排", &r(), NOW, TOP_K).is_empty());
    }

    #[test]
    fn 验收3_临时记忆过期后不影响回答() {
        let mut m = mk("t", "用户本周在赶项目压力大", MemoryType::TemporaryContext, Source::UserExplicit);
        m.expires_at = Some(NOW + 7 * DAY_MS);
        assert!(!retrieve(std::slice::from_ref(&m), "最近压力", &r(), NOW, TOP_K).is_empty());
        assert!(retrieve(&[m], "最近压力", &r(), NOW + 8 * DAY_MS, TOP_K).is_empty());
    }

    #[test]
    fn 验收4_闲聊不会污染记忆库() {
        let chats = ["今天好累啊", "你觉得呢", "6", "在吗", "晚安"];
        assert!(chats.iter().all(|c| parse_command(c).is_none()));
    }

    #[test]
    fn 验收6_低置信度推断不会被说成事实() {
        let mut guess = mk("g", "用户可能喜欢喝咖啡", MemoryType::Preference, Source::UserConfirmed);
        guess.confidence = 0.4;
        let hits = retrieve(&[guess], "咖啡", &r(), NOW, TOP_K);
        let text = render_context(&hits);
        assert!(text.contains("低置信度·未确认"), "没标出来：\n{text}");
        assert!(text.contains("不要把推断性记忆说成确定事实"));
    }
}
