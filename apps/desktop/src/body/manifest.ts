import { Manifest, MOOD_FALLBACK, clipKey, type Animat, type GraphClip, type GraphType, type Mood } from '@vpet/shared'

export const PET_BASE = '/pet'

/** 读取 build-assets 生成的 manifest.json 并做 schema 校验 */
export async function loadManifest(): Promise<Manifest> {
  const res = await fetch(`${PET_BASE}/manifest.json`)
  if (!res.ok) throw new Error(`manifest.json ${res.status}：先运行 pnpm build:assets`)
  return Manifest.parse(await res.json())
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

/** 某个类型下、在给定心情（含降级）可用的全部动画名字，如 idel 下的 amusement / aside / … */
export function namesFor(m: Manifest, type: GraphType, mood: Mood): string[] {
  const names = new Set<string>()
  const moods = new Set(MOOD_FALLBACK[mood])
  for (const c of m.clips) if (c.type === type && moods.has(c.mood)) names.add(c.name)
  return [...names]
}
