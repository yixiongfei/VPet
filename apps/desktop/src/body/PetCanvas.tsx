import { useCallback, useEffect, useRef, useState } from 'react'
import { AnimationPlayer } from './AnimationPlayer'
import { Bubble } from './Bubble'
import { ChatInput } from './ChatInput'
import { subscribe } from './events'
import { HitMask } from './hitMask'
import { Interaction } from './interaction'
import { loadManifest, loadProfile } from './manifest'
import { subscribePetState } from './petState'
import { pushHitMask } from './petWindow'
import { cannedReply, hideDelayMs } from './say'
import { toLogical } from './touch'

interface BubbleState {
  text: string
  streaming: boolean
}

export function PetCanvas() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const interactionRef = useRef<Interaction | null>(null)
  const sayGen = useRef(0)
  const hideTimer = useRef(0)
  const [error, setError] = useState<string | null>(null)
  const [size, setSize] = useState(500)
  const [name, setName] = useState('VPet')
  const [bubble, setBubble] = useState<BubbleState | null>(null)
  const [inputOpen, setInputOpen] = useState(false)

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    let disposed = false
    let player: AnimationPlayer | null = null
    const stops: Array<() => void> = []

    Promise.all([loadManifest(), loadProfile()])
      .then(([manifest, profile]) => {
        if (disposed) return
        setSize(manifest.size)
        setName(profile.name)
        player = new AnimationPlayer(canvas, manifest)
        // 每帧重算 alpha 掩码推给 Rust 的穿透判定（掩码没变就不推）
        const mask = new HitMask()
        player.onFrame = (composited) => {
          const changed = mask.update(composited)
          if (changed) void pushHitMask(changed)
        }
        const interaction = new Interaction({ player, manifest, profile })
        interactionRef.current = interaction
        interaction.start()
        stops.push(subscribePetState((s) => interaction.setState(s)))
        // Rust 侧全局快捷键按下时发这个事件（见 src-tauri/src/lib.rs）
        stops.push(subscribe('pet:prompt', () => setInputOpen(true)))
        console.info(
          `[VPet] ${profile.name} 载入：${manifest.clips.length} clips · ${manifest.size}px · ${manifest.generatedAt}`,
        )
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))

    return () => {
      disposed = true
      stops.forEach((f) => f())
      window.clearTimeout(hideTimer.current)
      interactionRef.current?.dispose()
      interactionRef.current = null
      player?.destroy()
    }
  }, [])

  const closeBubble = useCallback(() => {
    sayGen.current++ // 作废进行中的流
    window.clearTimeout(hideTimer.current)
    setBubble(null)
    interactionRef.current?.endSay()
  }, [])

  /** 流式把一段回话吐进气泡。Phase 3 把 cannedReply 换成 Brain 的 token 流即可 */
  const say = useCallback(async (userText: string) => {
    const gen = ++sayGen.current
    window.clearTimeout(hideTimer.current)
    interactionRef.current?.startSay()
    setBubble({ text: '', streaming: true })
    let acc = ''
    for await (const chunk of cannedReply(userText)) {
      if (gen !== sayGen.current) return
      acc += chunk
      setBubble({ text: acc, streaming: true })
    }
    if (gen !== sayGen.current) return
    setBubble({ text: acc, streaming: false })
    interactionRef.current?.endSay()
    hideTimer.current = window.setTimeout(() => {
      if (gen === sayGen.current) setBubble(null)
    }, hideDelayMs(acc))
  }, [])

  // Esc：先收输入框，再收气泡
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      if (inputOpen) setInputOpen(false)
      else if (bubble) closeBubble()
    }
    window.addEventListener('keydown', h)
    return () => window.removeEventListener('keydown', h)
  }, [inputOpen, bubble, closeBubble])

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
      onDoubleClick={() => setInputOpen(true)}
    >
      <canvas ref={canvasRef} />

      {(bubble || inputOpen) && (
        <div
          // 气泡在上、输入框在下，一起贴着窗口底部（docs/05 §4）
          style={{ position: 'absolute', left: 8, right: 8, bottom: 8, display: 'flex', flexDirection: 'column', gap: 8 }}
          // 别让点气泡/输入框的动作被宠物当成摸
          onPointerDown={(e) => e.stopPropagation()}
          onDoubleClick={(e) => e.stopPropagation()}
        >
          {bubble && <Bubble name={name} text={bubble.text} streaming={bubble.streaming} onClose={closeBubble} />}
          {inputOpen && <ChatInput onSubmit={(t) => void say(t)} onCancel={() => setInputOpen(false)} />}
        </div>
      )}

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
