import { z } from 'zod'
import { Mood } from './pet'

/**
 * 动画类型。与 legacy/VPet-Simulator.Core/Graph/GraphInfo.cs 的 GraphType 枚举同名（小写）。
 * 顺序有意义：build-assets 按此顺序做 token 序列匹配（原版逐个找第一个匹配的）。
 */
export const GRAPH_TYPES = [
  'common', 'raised_dynamic', 'raised_static', 'move', 'default', 'touch_head', 'touch_body',
  'idel', 'sleep', 'say', 'stateone', 'statetwo', 'startup', 'shutdown', 'work',
  'switch_up', 'switch_down', 'switch_thirsty', 'switch_hunger',
  'sidehide_left_main', 'sidehide_left_rise', 'sidehide_right_main', 'sidehide_right_rise',
] as const
export const GraphType = z.enum(GRAPH_TYPES)
export type GraphType = z.infer<typeof GraphType>

export const Animat = z.enum(['single', 'start', 'loop', 'end'])
export type Animat = z.infer<typeof Animat>

export const Frame = z.object({ src: z.string(), ms: z.number().int().positive() })
export type Frame = z.infer<typeof Frame>

export const GraphClip = z.object({
  id: z.string(),
  type: GraphType,
  name: z.string(),
  mood: Mood,
  animat: Animat,
  variant: z.number().int(),
  layer: z.enum(['main', 'back', 'front']).optional(),
  frames: z.array(Frame).min(1),
  totalMs: z.number().int(),
  /** 相对 assets-src/pet/vup 的源目录，便于追溯 */
  source: z.string(),
})
export type GraphClip = z.infer<typeof GraphClip>

/**
 * 食物精灵的一段轨迹。`visible: false` 表示这段时间食物不画
 * （原版 `a8#750` 只给时长就是这个意思）。坐标同样在 500×500 逻辑参考系里。
 */
export const FoodKeyframe = z.object({
  ms: z.number().positive(),
  visible: z.boolean(),
  x: z.number().optional(),
  y: z.number().optional(),
  width: z.number().optional(),
  rotate: z.number().optional(),
  opacity: z.number().optional(),
})
export type FoodKeyframe = z.infer<typeof FoodKeyframe>

/**
 * 夹心动画：后层（宠物本体）→ 食物精灵 → 前层（手）。
 * 移植自 legacy/VPet-Simulator.Core/Graph/FoodAnimation.cs——那里的注释写得很直白：
 * 「第二层夹心为运行时提供」。前后两层帧数不同但总时长相同，共用一个时钟。
 */
export const LayeredClip = z.object({
  id: z.string(),
  type: GraphType,
  name: z.string(),
  mood: Mood,
  animat: Animat,
  /** 后层 clip id（宠物本体） */
  back: z.string(),
  /** 前层 clip id（手，盖在食物上面） */
  front: z.string(),
  food: z.array(FoodKeyframe),
  source: z.string(),
})
export type LayeredClip = z.infer<typeof LayeredClip>

/**
 * 食物：夹心动画中间那层的图 + Phase 2 状态机要用的营养值。
 * 原版的价格/经验/好感度是养成经济，这个产品里没有，不带过来。
 */
export const FoodItem = z.object({
  id: z.string(),
  name: z.string(),
  /** eat / drink / gift —— 决定配哪段夹心动画 */
  graph: z.string(),
  /** Meal / Snack / Drink / Drug / Gift / Functional */
  type: z.string(),
  src: z.string(),
  strength: z.number(),
  /** 回多少饱腹 */
  strengthFood: z.number(),
  /** 回多少水 */
  strengthDrink: z.number(),
  feeling: z.number(),
  health: z.number(),
})
export type FoodItem = z.infer<typeof FoodItem>

export const Manifest = z.object({
  pet: z.string(),
  size: z.number().int(),
  generatedAt: z.string(),
  clips: z.array(GraphClip),
  /** "type/name/mood/animat" → clip ids（同键多个 = 随机变体） */
  index: z.record(z.string(), z.array(z.string())),
  /** 夹心动画。数量很少（吃/喝/收礼 × 心情），查找直接线性找 */
  layered: z.array(LayeredClip).default([]),
  food: z.array(FoodItem).default([]),
})
export type Manifest = z.infer<typeof Manifest>

export const clipKey = (type: GraphType, name: string, mood: Mood, animat: Animat) =>
  `${type}/${name}/${mood}/${animat}`
