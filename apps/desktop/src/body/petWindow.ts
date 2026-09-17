import { invokeCore } from './ipc'

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
export const giveGift = (id?: string) => invokeCore<string>('give_gift', { id })

/**
 * 按逻辑像素平移宠物窗口——提起时用来让窗口跟住光标。
 * 浏览器预览里没有 Tauri，静默跳过（动画照常，只是窗口不动）。
 */
// Serialize begin/end so a quick release cannot overtake an async begin IPC.
let dragQueue: Promise<unknown> = Promise.resolve()
export function beginPetDrag(anchorX: number, anchorY: number): void {
  dragQueue = dragQueue.then(() => invokeCore('begin_pet_drag', { anchorX, anchorY }))
}
export function endPetDrag(): void {
  dragQueue = dragQueue.then(() => invokeCore('end_pet_drag'))
}

export const openChat = () => invokeCore('open_chat')
export const openSettingsPanel = () => invokeCore('open_settings_panel')
