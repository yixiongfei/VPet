const IS_TAURI = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

type WindowApi = typeof import('@tauri-apps/api/window')
type DpiApi = typeof import('@tauri-apps/api/dpi')

let api: Promise<[WindowApi, DpiApi]> | null = null
const loadApi = () => (api ??= Promise.all([import('@tauri-apps/api/window'), import('@tauri-apps/api/dpi')]))

let invoker: Promise<typeof import('@tauri-apps/api/core')> | null = null
const loadCore = () => (invoker ??= import('@tauri-apps/api/core'))

async function call(cmd: string, args: Record<string, unknown>): Promise<void> {
  if (!IS_TAURI) return
  try {
    const { invoke } = await loadCore()
    await invoke(cmd, args)
  } catch (e) {
    console.warn(`[VPet] ${cmd} 失败`, e)
  }
}

/** 把当前帧的命中掩码推给 Rust 的穿透判定（按位打包的 48×48） */
export const pushHitMask = (cells: Uint8Array) => call('set_hit_mask', { cells: Array.from(cells) })

/**
 * 交互期间钉住窗口不穿透。不钉的话，提起后把宠物拖到光标不再压着它的位置时，
 * 轮询会把窗口切成穿透，拖拽当场断掉。
 */
export const setHitTestPinned = (pinned: boolean) => call('set_hit_test_pinned', { pinned })

/**
 * 按逻辑像素平移宠物窗口——提起时用来让窗口跟住光标。
 * 浏览器预览里没有 Tauri，静默跳过（动画照常，只是窗口不动）。
 */
export async function moveWindowBy(dx: number, dy: number): Promise<void> {
  if (!IS_TAURI || (dx === 0 && dy === 0)) return
  try {
    const [{ getCurrentWindow }, { LogicalPosition }] = await loadApi()
    const win = getCurrentWindow()
    const scale = await win.scaleFactor()
    const pos = (await win.outerPosition()).toLogical(scale)
    await win.setPosition(new LogicalPosition(pos.x + dx, pos.y + dy))
  } catch (e) {
    console.warn('[VPet] 移动窗口失败', e)
  }
}
