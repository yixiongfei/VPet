import { useEffect, useRef, useState } from 'react'
import type { Mood } from '@vpet/shared'
import { AnimationPlayer } from './AnimationPlayer'
import { loadManifest, namesFor, pick } from './manifest'

const IS_TAURI = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/** Phase 0：默认心情。Phase 2 起改为订阅 Core 的 `pet:state` 事件。 */
const MOOD: Mood = 'nomal'

/** 空闲小动作：呼吸若干秒后随机播一段 idel（原版 duration 表里 boring/squat 约 20s） */
const IDLE_ACTION_EVERY_MS: [number, number] = [15_000, 40_000]

export function PetCanvas() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const playerRef = useRef<AnimationPlayer | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [size, setSize] = useState(500)

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    let disposed = false
    let idleTimer = 0

    const scheduleIdleAction = (player: AnimationPlayer, idleNames: string[]) => {
      if (idleNames.length === 0) return
      const [lo, hi] = IDLE_ACTION_EVERY_MS
      idleTimer = window.setTimeout(() => {
        if (disposed) return
        void player.playOnce({ type: 'idel', name: pick(idleNames), mood: MOOD })
      }, lo + Math.random() * (hi - lo))
    }

    loadManifest()
      .then((manifest) => {
        if (disposed) return
        setSize(manifest.size)
        const player = new AnimationPlayer(canvas, manifest)
        playerRef.current = player
        const idleNames = namesFor(manifest, 'idel', MOOD)
        // 任何一段一次性动画结束 → 回到呼吸，并安排下一次小动作
        player.onIdle = () => {
          if (disposed) return
          void player.play({ type: 'default', mood: MOOD })
          scheduleIdleAction(player, idleNames)
        }
        void player.play({ type: 'default', mood: MOOD })
        scheduleIdleAction(player, idleNames)
        console.info(`[VPet] manifest 载入：${manifest.clips.length} clips · ${manifest.size}px · ${manifest.generatedAt}`)
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))

    return () => {
      disposed = true
      window.clearTimeout(idleTimer)
      playerRef.current?.destroy()
      playerRef.current = null
    }
  }, [])

  /** 按住拖动 → 移动整个窗口（Tauri 内）；浏览器里预览时无效 */
  const onPointerDown = async (e: React.PointerEvent) => {
    if (e.button !== 0 || !IS_TAURI) return
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      await getCurrentWindow().startDragging()
    } catch (err) {
      console.warn('startDragging 失败', err)
    }
  }

  /** 双击 → 摸头（三段式 start/loop/end 的演示；Phase 1 会换成按触摸区域判定） */
  const onDoubleClick = () => {
    void playerRef.current?.playOnce({ type: 'touch_head', mood: MOOD })
  }

  return (
    <div
      style={{ width: size, height: size, position: 'relative' }}
      onPointerDown={onPointerDown}
      onDoubleClick={onDoubleClick}
    >
      <canvas ref={canvasRef} />
      {error && (
        <div
          style={{
            position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', padding: 24, textAlign: 'center',
            font: '14px/1.6 system-ui, sans-serif', color: '#fff', background: 'rgba(0,0,0,.72)', borderRadius: 16,
          }}
        >
          <div>
            <div style={{ fontSize: 28, marginBottom: 8 }}>(´・ω・`)</div>
            <div>{error}</div>
          </div>
        </div>
      )}
    </div>
  )
}
