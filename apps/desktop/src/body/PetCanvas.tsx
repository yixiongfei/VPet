import { Verdict } from '@vpet/shared'
import { useCallback, useEffect, useRef, useState } from 'react'
import { AnimationPlayer } from './AnimationPlayer'
import { Bubble } from './Bubble'
import { subscribe } from './events'
import { HitMask } from './hitMask'
import { Interaction } from './interaction'
import { loadManifest, loadProfile } from './manifest'
import { fetchPetState, subscribePetState } from './petState'
import { openChat, openSettingsPanel, pushFoodCatalog, pushHitMask, reportTouch } from './petWindow'
import { hideDelayMs } from './say'
import { toLogical } from './touch'

export function PetCanvas() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const maskRef = useRef<HitMask | null>(null)
  const interactionRef = useRef<Interaction | null>(null)
  const hideTimer = useRef(0)
  const [error, setError] = useState<string | null>(null)
  const [name, setName] = useState('VPet')
  const [bubble, setBubble] = useState<string | null>(null)
  const [streaming, setStreaming] = useState(false)
  const streamRef = useRef<{ id: string; text: string } | null>(null)
  const [hovered, setHovered] = useState(false)
  const [dragging, setDragging] = useState(false)

  const announce = useCallback((text: string) => {
    window.clearTimeout(hideTimer.current)
    streamRef.current = null
    setStreaming(false)
    setBubble(text)
    hideTimer.current = window.setTimeout(() => setBubble(null), hideDelayMs(text))
  }, [])

  /** 聊天窗口里她在说什么，桌面上的她也同步「说」出来（流式气泡） */
  const onChatStream = useCallback((payload: unknown) => {
    const event = payload as { requestId?: string; delta?: string; done?: boolean; text?: string } | null
    if (!event?.requestId) return
    const current = streamRef.current
    if (event.done) {
      if (current?.id !== event.requestId) return
      streamRef.current = null
      setStreaming(false)
      // Core 收尾时会把清理过的最终文本带回来（去掉模型拖出的旁白），以它为准
      const text = (event.text ?? current.text).trim()
      if (!text) { setBubble(null); return }
      setBubble(text)
      hideTimer.current = window.setTimeout(() => setBubble(null), hideDelayMs(text))
      return
    }
    window.clearTimeout(hideTimer.current)
    const next = current?.id === event.requestId
      ? { id: event.requestId, text: current.text + (event.delta ?? '') }
      : { id: event.requestId, text: event.delta ?? '' }
    streamRef.current = next
    setStreaming(true)
    setBubble(next.text)
  }, [])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    let disposed = false
    let player: AnimationPlayer | null = null
    const stops: Array<() => void> = []

    Promise.all([loadManifest(), loadProfile()])
      .then(([manifest, profile]) => {
        if (disposed) return
        setName(profile.name)
        player = new AnimationPlayer(canvas, manifest)
        // Preserve source resolution; fit viewport at every desktop size.
        canvas.style.width = '100%'
        canvas.style.height = '100%'
        const mask = new HitMask()
        maskRef.current = mask
        player.onFrame = (composited) => {
          const changed = mask.update(composited)
          if (changed) pushHitMask(changed)
        }
        const interaction = new Interaction({
          player, manifest, profile,
          onTouch: reportTouch,
          onClick: () => void openChat(),
          onDragChange: setDragging,
        })
        interactionRef.current = interaction
        interaction.start()
        pushFoodCatalog(manifest.food)
        stops.push(subscribePetState((state) => interaction.setState(state)))
        void fetchPetState().then((state) => { if (state && !disposed) interaction.setState(state) })
        stops.push(subscribe('pet:prompt', () => void openChat()))
        stops.push(subscribe('chat-stream', onChatStream))
        stops.push(subscribe('pet:drag-ended', () => interaction.onPointerUp(true)))
        stops.push(subscribe('pet:gift', (payload) => {
          const received = payload as { say?: string }
          if (received?.say) announce(received.say)
        }))
        stops.push(subscribe('timer:fired', (payload) => {
          const label = (payload as { label?: string } | null)?.label
          if (label) announce('⏰ ' + label)
        }))
        stops.push(subscribe('pet:said', (payload) => {
          const result = Verdict.safeParse(payload)
          if (result.success) announce(result.data.say)
        }))
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))

    const cancel = () => interactionRef.current?.onPointerUp(true)
    window.addEventListener('blur', cancel)
    return () => {
      disposed = true
      stops.forEach((stop) => stop())
      window.removeEventListener('blur', cancel)
      window.clearTimeout(hideTimer.current)
      interactionRef.current?.dispose()
      interactionRef.current = null
      maskRef.current = null
      player?.destroy()
    }
  }, [announce, onChatStream])

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return
    const point = toLogical(event.currentTarget, event.clientX, event.clientY)
    if (maskRef.current && !maskRef.current.isOpaqueAt(point.x, point.y)) return
    event.currentTarget.setPointerCapture(event.pointerId)
    interactionRef.current?.onPointerDown(point.x, point.y, event.screenX, event.screenY)
  }

  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const point = toLogical(event.currentTarget, event.clientX, event.clientY)
    setHovered(maskRef.current?.isOpaqueAt(point.x, point.y) ?? true)
    interactionRef.current?.onPointerMove(point.x, point.y, event.screenX, event.screenY)
  }

  const onPointerUp = (event: React.PointerEvent<HTMLDivElement>) => {
    interactionRef.current?.onPointerUp()
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId)
  }

  return (
    <div
      style={{ width: '100%', height: '100%', position: 'relative', touchAction: 'none', cursor: dragging ? 'grabbing' : hovered ? 'pointer' : 'default' }}
      title="单击聊天 · 按住拖动 · 右键打开设置"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => interactionRef.current?.onPointerUp(true)}
      onLostPointerCapture={() => interactionRef.current?.onPointerUp(true)}
      onPointerLeave={() => setHovered(false)}
      onContextMenu={(event) => { event.preventDefault(); void openSettingsPanel() }}
    >
      <canvas ref={canvasRef} />
      {bubble && (
        <div style={{ position: 'absolute', left: 8, right: 8, bottom: 8, pointerEvents: 'none' }}>
          <Bubble name={name} text={bubble} streaming={streaming} onClose={() => setBubble(null)} />
        </div>
      )}
      {error && (
        <div style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', padding: 24, textAlign: 'center', font: '14px/1.6 system-ui, sans-serif', color: '#fff', background: 'rgba(0,0,0,.72)', borderRadius: 16 }}>
          <div><div style={{ fontSize: 28, marginBottom: 8 }}>VPet</div>{error}</div>
        </div>
      )}
    </div>
  )
}
