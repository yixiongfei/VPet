import {
  Manifest, MOOD_FALLBACK, clipKey, parsePetProfile,
  type Animat, type FoodItem, type GraphClip, type GraphType, type LayeredClip, type Mood, type PetProfile,
} from '@vpet/shared'

export const PET_BASE = '/pet'

/** 读取 build-assets 生成的 manifest.json 并做 schema 校验 */
export async function loadManifest(): Promise<Manifest> {
  const res = await fetch(`${PET_BASE}/manifest.json`)
  if (!res.ok) throw new Error(`manifest.json ${res.status}：先运行 pnpm build:assets`)
  return Manifest.parse(await res.json())
}

/** 读取 vup.lps 转出的 pet.json：触摸区域、提起锚点 */
export async function loadProfile(): Promise<PetProfile> {
  const res = await fetch(`${PET_BASE}/pet.json`)
  if (!res.ok) throw new Error(`pet.json ${res.status}：先运行 pnpm build:assets`)
  return parsePetProfile(await res.json())
}

/**
 * 找 (type, name, mood, animat) 对应的全部变体；请求的心情没有时按 MOOD_FALLBACK 降级
 * （与原版 GraphCore.FindGraphs 的行为一致）。
 */
export function resolveClips(m: Manifest, type: GraphType, name: string, mood: Mood, animat: Animat): GraphClip[] {
  for (const fb of MOOD_FALLBACK[mood]) {
    const ids = m.index[clipKey(type, name, fb, animat)]
    if (ids?.length) return ids.map((id) => byId(m, id)).filter((c): c is GraphClip => !!c)
  }
  return []
}

/**
 * 找 (type, name, mood) 对应的夹心动画（吃 / 喝 / 收礼），同样按 MOOD_FALLBACK 降级。
 * 夹心动画总共十来段，直接线性找，不值得建索引。
 */
export function resolveLayered(m: Manifest, type: GraphType, name: string, mood: Mood): LayeredClip | undefined {
  for (const fb of MOOD_FALLBACK[mood]) {
    const hit = m.layered.find((l) => l.type === type && l.name === name && l.mood === fb)
    if (hit) return hit
  }
  return undefined
}

/**
 * 给一段夹心动画挑一样食物。`graph` 就是夹心动画的名字（eat / drink / gift）。
 * 排掉 Drug——那是原版用来救存档的药，`太阳系` 一口下去体力 −100，自发进食不该吃它。
 * Phase 2 起改由 Core 决定吃什么（吃什么决定回多少饱腹）。
 */
export function pickFood(m: Manifest, graph: string): FoodItem | undefined {
  const usable = m.food.filter((f) => f.graph === graph && f.type !== 'Drug')
  return usable.length ? pick(usable) : undefined
}

const idCache = new WeakMap<Manifest, Map<string, GraphClip>>()
export function byId(m: Manifest, id: string): GraphClip | undefined {
  let map = idCache.get(m)
  if (!map) {
    map = new Map(m.clips.map((c) => [c.id, c]))
    idCache.set(m, map)
  }
  return map.get(id)
}

export const pick = <T>(arr: T[]): T => arr[Math.floor(Math.random() * arr.length)]

/**
 * 某个类型下、按心情就近可用的动画名字，如 idel 下的 amusement / aside / …
 *
 * 与原版 GraphCore.FindName 有一处有意的出入：原版的名字表是心情无关的
 * （GraphCore.cs:88-97），随机挑中一个只在 ill 下画过的名字时，心情正常也会播生病的
 * 动画——摸身体就有这个问题（4 个名字里 touch_body 只有 ill，tb1/tb2/turn 只有 happy）。
 * 这里改成按 MOOD_FALLBACK 找第一个有动画的心情，只在那个心情里挑名字。
 */
export function namesFor(m: Manifest, type: GraphType, mood: Mood): string[] {
  for (const fb of MOOD_FALLBACK[mood]) {
    const names = new Set<string>()
    for (const c of m.clips) if (c.type === type && c.mood === fb) names.add(c.name)
    if (names.size) return [...names]
  }
  return []
}
