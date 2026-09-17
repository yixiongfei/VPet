import { z } from 'zod'

/** 与 Rust 侧 core/state_machine.rs 的 Mood 一一对应，也与资产目录 Happy/Nomal/PoorCondition/Ill 对应 */
export const Mood = z.enum(['happy', 'nomal', 'poorcondition', 'ill'])
export type Mood = z.infer<typeof Mood>

export const Activity = z.enum([
  'idle', 'working', 'break', 'studying', 'sleeping', 'playing',
  /** 饱腹/口渴掉到阈值以下时由 Core 触发，播夹心动画（后层宠物 → 食物 → 前层手） */
  'eating', 'drinking',
  /** 收礼物。由用户从托盘触发，她自己不会凭空收到 */
  'gift',
])
export type Activity = z.infer<typeof Activity>

/** 正在做的事。`graph` 决定播哪段动画，`reason` 说明她为什么选了它 */
export const ActionRef = z.object({
  id: z.string(),
  name: z.string(),
  graph: z.string(),
  reason: z.string(),
  /** 吃 / 喝 时她买下的那一样东西，Body 照这个 id 渲染精灵 */
  food: z.object({ id: z.string(), name: z.string() }).nullish(),
})
export type ActionRef = z.infer<typeof ActionRef>

/** Core → Body 的 `pet:state` 事件载荷 */
export const PetState = z.object({
  activity: Activity,
  mood: Mood,
  strength: z.number().min(0).max(100),
  feeling: z.number().min(0).max(100),
  /** 饱腹度：低了自己去吃东西 */
  hunger: z.number().min(0).max(100),
  /** 口渴度：低了自己去喝水 */
  thirst: z.number().min(0).max(100),
  /** 工作挣的钱，用来买东西（Core 的动作表决定挣多少） */
  money: z.number().min(0),
  /** 学习涨的经验，决定等级 */
  exp: z.number().min(0),
  /** 等级解锁更赚钱的活，也直接给收入加成 */
  level: z.number().int().min(0),
  /**
   * 好感度。**慢变量**——天级才看得出变化，和分钟级的 feeling 分属两个时间尺度。
   * 决定她有多大概率听你的话：今天心情差可以不听，但关系好的话拒绝得更软
   */
  affection: z.number().min(0).max(100),
  action: ActionRef.nullish(),
  updatedAt: z.number().int(),
})
export type PetState = z.infer<typeof PetState>

/** 心情降级顺序：请求的心情没有对应动画时按此顺序找最近的 */
export const MOOD_FALLBACK: Record<Mood, Mood[]> = {
  happy: ['happy', 'nomal', 'poorcondition', 'ill'],
  nomal: ['nomal', 'happy', 'poorcondition', 'ill'],
  poorcondition: ['poorcondition', 'nomal', 'ill', 'happy'],
  ill: ['ill', 'poorcondition', 'nomal', 'happy'],
}

/** 她为什么不干。每一项对应一句固定台词，不需要生成模型 */
export const Refusal = z.enum([
  /** 做不到（等级/前置条件不够），不是不听话 */
  'impossible',
  /** 压根没有这件事可做 */
  'unknown',
  'hungry', 'thirsty', 'tired', 'sad',
  /** 没有哪项特别突出，就是不太想 */
  'reluctant',
])
export type Refusal = z.infer<typeof Refusal>

/**
 * Core → Body 的 `pet:said` 事件载荷：一次服从判定的结果。
 *
 * `p` 是这次服从的概率，一并带出来是为了能显示「刚才那次有 63% 会听」——
 * 概率系统不给人看就成了玄学。
 */
export const Verdict = z.object({
  obey: z.boolean(),
  p: z.number().min(0).max(1),
  /** 被要求的那件事 */
  action: z.string(),
  refusal: Refusal.nullish(),
  /** 直接能显示在气泡里的一句话 */
  say: z.string(),
})
export type Verdict = z.infer<typeof Verdict>

/* ==================== 长期互动记忆（roadmap 2.11） ==================== */

/** 记忆类型。决定它的默认有效期和在上下文里的标签 */
export const MemoryType = z.enum([
  /** 稳定身份与目标：「准备 2027 考研」 */
  'profile',
  /** 偏好：「学习时不喜欢频繁打扰」 */
  'preference',
  /** 习惯：「晚上工作，白天学习」 */
  'habit',
  /** 临时状态：「本周在赶项目」。自带有效期 */
  'temporary_context',
  /** 角色互动摘要：「送过礼物，好感上升」 */
  'relationship',
  /** 约定或计划：「明天提醒复习行列式」。自带截止日期 */
  'commitment',
])
export type MemoryType = z.infer<typeof MemoryType>

/**
 * 这条记忆是怎么来的。**冲突时的优先级就是这个顺序**（从高到低）：
 * user_explicit > user_confirmed > system_event > inferred。
 * 未确认的推断永远不进模型上下文。
 */
export const MemorySource = z.enum(['user_explicit', 'user_confirmed', 'system_event', 'inferred'])
export type MemorySource = z.infer<typeof MemorySource>

/** active 才进上下文；archived 可恢复；deleted 是软删除，行留着当审计 */
export const MemoryStatus = z.enum(['active', 'archived', 'deleted'])
export type MemoryStatus = z.infer<typeof MemoryStatus>

export const MemoryItem = z.object({
  id: z.string(),
  /** 简洁、可读的记忆摘要。**不是原始对话** */
  content: z.string(),
  type: MemoryType,
  importance: z.number().min(0).max(100),
  confidence: z.number().min(0).max(1),
  source: MemorySource,
  /** 置顶的永不自动淘汰，且不管问什么都进候选 */
  pinned: z.boolean(),
  createdAt: z.number().int(),
  updatedAt: z.number().int(),
  lastAccessedAt: z.number().int(),
  accessCount: z.number().int().min(0),
  expiresAt: z.number().int().nullish(),
  status: MemoryStatus,
  /** 算相似度用的是哪一版表示。换模型后按它筛出要重算的 */
  embeddingVersion: z.string(),
  /** 被取代的上一版。那条链就是「这条记忆怎么变成今天这样」的审计 */
  parentId: z.string().nullish(),
})
export type MemoryItem = z.infer<typeof MemoryItem>

/**
 * 一次检索命中。三个分数都带出来，是为了排序能被人检查——
 * 融合的黑盒不给人看就成了玄学。
 * `lexical` 是字面（字符 n-gram 余弦），`dense` 是语义（向量最近邻），
 * `similarity` 是两者的加权和，也就是打分公式里的那一项。
 */
export const MemoryHit = z.object({
  item: MemoryItem,
  similarity: z.number().min(0).max(1),
  lexical: z.number().min(0).max(1),
  /** null = 这条不在向量最近邻里，或者模型没就绪 */
  dense: z.number().min(0).max(1).nullish(),
  score: z.number(),
})
export type MemoryHit = z.infer<typeof MemoryHit>

/** 记忆库健康度。`usedRatio` 低说明检索被一堆没人看的旧条目稀释了 */
export const MemoryHealth = z.object({
  active: z.number().int(),
  archived: z.number().int(),
  deleted: z.number().int(),
  expired: z.number().int(),
  usedRatio: z.number().min(0).max(1),
  avgImportance: z.number(),
  needsSweep: z.number().int(),
})
export type MemoryHealth = z.infer<typeof MemoryHealth>

/**
 * 语义模型的状态。**悄悄降级是最坏的一种降级**——
 * 用户得能看到「现在到底在用哪种检索」。
 */
export const EmbedState = z.discriminatedUnion('state', [
  /** 没配模型，纯字面检索 */
  z.object({ state: z.literal('disabled'), reason: z.string() }),
  z.object({ state: z.literal('loading') }),
  z.object({ state: z.literal('ready'), name: z.string(), dim: z.number().int() }),
  /** 试过了起不来，带上原因 */
  z.object({ state: z.literal('failed'), reason: z.string() }),
])
export type EmbedState = z.infer<typeof EmbedState>
