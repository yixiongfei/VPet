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

export const Manifest = z.object({
  pet: z.string(),
  size: z.number().int(),
  generatedAt: z.string(),
  clips: z.array(GraphClip),
  /** "type/name/mood/animat" → clip ids（同键多个 = 随机变体） */
  index: z.record(z.string(), z.array(z.string())),
})
export type Manifest = z.infer<typeof Manifest>

export const clipKey = (type: GraphType, name: string, mood: Mood, animat: Animat) =>
  `${type}/${name}/${mood}/${animat}`
