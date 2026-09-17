import { IS_TAURI, invokeCore } from './ipc'

type WindowApi = typeof import('@tauri-apps/api/window')
type DpiApi = typeof import('@tauri-apps/api/dpi')

let api: Promise<[WindowApi, DpiApi]> | null = null
const loadApi = () => (api ??= Promise.all([import('@tauri-apps/api/window'), import('@tauri-apps/api/dpi')]))

/** 把当前帧的命中掩码推给 Rust 的穿透判定（按位打包的 48×48） */
export const pushHitMask = (cells: Uint8Array) => void invokeCore('set_hit_mask', { cells: Array.from(cells) })

/**
 * 交互期间钉住窗口不穿透。不钉的话，提起后把宠物拖到光标不再压着它的位置时，
 * 轮询会把窗口切成穿透，拖拽当场断掉。
 */
export const setHitTestPinned = (pinned: boolean) => void invokeCore('set_hit_test_pinned', { pinned })

/** 报告宠物被摸了。数值怎么变是 Core 状态机的事，Body 不自己算 */
export const reportTouch = (zone: string) => void invokeCore('pet_touched', { zone })

/**
 * 把食物目录交给 Core。食物数据来自 manifest（build-assets 从原版转出的 123 项），
 * 但「买哪样」是状态机的决定——它得先看得见这张表。
 */
export const pushFoodCatalog = (items: unknown[]) => void invokeCore('set_food_catalog', { items })

/** 送她一样礼物（随机挑一件）。返回礼物名字，没货架时返回 null */
export const giveGift = () => invokeCore<string>('give_gift')

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
