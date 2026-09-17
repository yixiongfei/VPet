const IS_TAURI = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/**
 * 订阅 Core 发来的事件（docs/03 §5 事件面）。
 *
 * 浏览器预览里没有 Tauri，退化成监听同名的 window CustomEvent，这样不开 Tauri
 * 也能调：`window.dispatchEvent(new CustomEvent('pet:state', { detail: {…} }))`
 */
export function subscribe(name: string, onPayload: (payload: unknown) => void): () => void {
  if (!IS_TAURI) {
    const h = (e: Event) => onPayload((e as CustomEvent<unknown>).detail)
    window.addEventListener(name, h)
    return () => window.removeEventListener(name, h)
  }
  let stop: (() => void) | null = null
  let cancelled = false
  void import('@tauri-apps/api/event')
    .then(({ listen }) => listen(name, (e) => onPayload(e.payload)))
    .then((un) => {
      if (cancelled) un()
      else stop = un
    })
    .catch((e: unknown) => console.warn(`[VPet] 订阅 ${name} 失败`, e))
  return () => {
    cancelled = true
    stop?.()
  }
}
