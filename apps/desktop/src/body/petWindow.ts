const IS_TAURI = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

type WindowApi = typeof import('@tauri-apps/api/window')
type DpiApi = typeof import('@tauri-apps/api/dpi')

let api: Promise<[WindowApi, DpiApi]> | null = null
const loadApi = () => (api ??= Promise.all([import('@tauri-apps/api/window'), import('@tauri-apps/api/dpi')]))

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
