import {
  Manifest, MOOD_FALLBACK, clipKey, parsePetProfile,
  type Animat, type GraphClip, type GraphType, type Mood, type PetProfile,
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

const idCache = new WeakMap<Manifest, Map<string, GraphClip>>()
function byId(m: Manifest, id: string): GraphClip | undefined {
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
