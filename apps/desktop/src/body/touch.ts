import { PET_LOGICAL_SIZE, type Mood, type PetProfile, type Point, type Rect } from '@vpet/shared'

/** 短按能命中的区域（原版 TouchArea.IsPress = false，见 legacy Main.xaml.cs:286-287） */
export type ClickZone = 'head' | 'body'
/** 长按能命中的区域（原版 TouchArea.IsPress = true，见 legacy Main.xaml.cs:288-302） */
export type PressZone = 'raise'

const inside = (r: Rect, x: number, y: number) =>
  x >= r.px && x <= r.px + r.sw && y >= r.py && y <= r.py + r.sh

/** 按原版 TouchEvent 的注册顺序判定：先头、后身体 */
export function clickZone(p: PetProfile, x: number, y: number): ClickZone | null {
  if (inside(p.touchHead, x, y)) return 'head'
  if (inside(p.touchBody, x, y)) return 'body'
  return null
}

/** 提起区域按心情不同，只判当前心情那一个（原版给 4 种心情各注册一个 TouchArea，命中后校验 Mode） */
export function pressZone(p: PetProfile, mood: Mood, x: number, y: number): PressZone | null {
  return inside(p.touchRaised[mood], x, y) ? 'raise' : null
}

/** DOM 坐标 → pet.json 的 500×500 参考系（元素被 CSS 缩放也成立） */
export function toLogical(el: Element, clientX: number, clientY: number): Point {
  const r = el.getBoundingClientRect()
  return {
    x: ((clientX - r.left) / r.width) * PET_LOGICAL_SIZE,
    y: ((clientY - r.top) / r.height) * PET_LOGICAL_SIZE,
  }
}
