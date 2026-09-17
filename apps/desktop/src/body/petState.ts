import { PetState, type Activity, type GraphType } from '@vpet/shared'
import { subscribe } from './events'

/** Core 还没发过状态时的兜底；Phase 2 起由 Rust 的状态机接管 */
export const DEFAULT_PET_STATE: PetState = {
  activity: 'idle',
  mood: 'nomal',
  strength: 100,
  feeling: 60,
  hunger: 100,
  thirst: 100,
  updatedAt: 0,
}

/**
 * 活动 → 该活动待机时循环播放的动画（docs/05 §3）。
 * 不写 name 的表示「该类型下按心情随机挑一个」，如 idle 的 default、break 的各种 idel。
 */
export const CLIP_FOR: Record<Activity, { type: GraphType; name?: string }> = {
  idle: { type: 'default' },
  working: { type: 'work', name: 'workone' },
  studying: { type: 'work', name: 'study' },
  break: { type: 'idel' },
  sleeping: { type: 'sleep' },
  playing: { type: 'work', name: 'playone' },
  // 夹心动画：AnimationPlayer 认出来后会拉起后层 + 前层两条轨道
  eating: { type: 'common', name: 'eat' },
  drinking: { type: 'common', name: 'drink' },
}

/** 订阅 Core 的 `pet:state`，载荷过一遍 zod，不合法只 warn 不崩 */
export function subscribePetState(onState: (s: PetState) => void): () => void {
  return subscribe('pet:state', (payload) => {
    const parsed = PetState.safeParse(payload)
    if (parsed.success) onState(parsed.data)
    else console.warn('[VPet] pet:state 载荷不合法', parsed.error.issues)
  })
}
