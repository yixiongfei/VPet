import { useEffect, useRef, useState } from 'react'
import type { Mood } from '@vpet/shared'
import { AnimationPlayer } from './AnimationPlayer'
import { Interaction } from './interaction'
import { loadManifest, loadProfile } from './manifest'
import { toLogical } from './touch'

/** Phase 0：默认心情。Phase 2 起改为订阅 Core 的 `pet:state` 事件。 */
const MOOD: Mood = 'nomal'

export function PetCanvas() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const interactionRef = useRef<Interaction | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [size, setSize] = useState(500)

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    let disposed = false
    let player: AnimationPlayer | null = null

    Promise.all([loadManifest(), loadProfile()])
      .then(([manifest, profile]) => {
        if (disposed) return
        setSize(manifest.size)
        player = new AnimationPlayer(canvas, manifest)
        const interaction = new Interaction({ player, manifest, profile, mood: () => MOOD })
        interactionRef.current = interaction
        interaction.start()
        console.info(
          `[VPet] ${profile.name} 载入：${manifest.clips.length} clips · ${manifest.size}px · ${manifest.generatedAt}`,
        )
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))

    return () => {
      disposed = true
      interactionRef.current?.dispose()
      interactionRef.current = null
      player?.destroy()
    }
  }, [])

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return
    // 捕获指针：提起后光标移出窗口也还能收到 move / up
    e.currentTarget.setPointerCapture(e.pointerId)
    const p = toLogical(e.currentTarget, e.clientX, e.clientY)
    interactionRef.current?.onPointerDown(p.x, p.y)
  }

  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const p = toLogical(e.currentTarget, e.clientX, e.clientY)
    interactionRef.current?.onPointerMove(p.x, p.y)
  }

  const onPointerUp = () => interactionRef.current?.onPointerUp()

  return (
    <div
      style={{ width: size, height: size, position: 'relative', touchAction: 'none' }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
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
