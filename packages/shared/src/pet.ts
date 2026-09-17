import { z } from 'zod'

/** 与 Rust 侧 core/state_machine.rs 的 Mood 一一对应，也与资产目录 Happy/Nomal/PoorCondition/Ill 对应 */
export const Mood = z.enum(['happy', 'nomal', 'poorcondition', 'ill'])
export type Mood = z.infer<typeof Mood>

export const Activity = z.enum([
  'idle', 'working', 'break', 'studying', 'sleeping', 'playing',
  /** 饱腹/口渴掉到阈值以下时由 Core 触发，播夹心动画（后层宠物 → 食物 → 前层手） */
  'eating', 'drinking',
])
export type Activity = z.infer<typeof Activity>

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
