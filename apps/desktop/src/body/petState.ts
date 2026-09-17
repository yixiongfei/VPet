import { PetState, type Activity, type GraphType } from '@vpet/shared'

const IS_TAURI = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/** Core 还没发过状态时的兜底；Phase 2 起由 Rust 的状态机接管 */
export const DEFAULT_PET_STATE: PetState = {
  activity: 'idle',
  mood: 'nomal',
  strength: 100,
  feeling: 60,
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
}

/**
 * 订阅 Core 的 `pet:state`（docs/03 §5 事件面）。
 * 浏览器预览里没有 Tauri，退化成监听同名的 window CustomEvent，
 * 这样不开 Tauri 也能调状态：
 *
 *   window.dispatchEvent(new CustomEvent('pet:state', { detail: { activity: 'sleeping', … } }))
 */
export function subscribePetState(onState: (s: PetState) => void): () => void {
  if (!IS_TAURI) {
    const h = (e: Event) => handle((e as CustomEvent<unknown>).detail, onState)
    window.addEventListener('pet:state', h)
    return () => window.removeEventListener('pet:state', h)
  }
  let stop: (() => void) | null = null
  let cancelled = false
  void import('@tauri-apps/api/event')
    .then(({ listen }) => listen('pet:state', (e) => handle(e.payload, onState)))
    .then((un) => {
      if (cancelled) un()
      else stop = un
    })
    .catch((e: unknown) => console.warn('[VPet] 订阅 pet:state 失败', e))
  return () => {
    cancelled = true
    stop?.()
  }
}

function handle(payload: unknown, onState: (s: PetState) => void): void {
  const parsed = PetState.safeParse(payload)
  if (parsed.success) onState(parsed.data)
  else console.warn('[VPet] pet:state 载荷不合法', parsed.error.issues)
}
