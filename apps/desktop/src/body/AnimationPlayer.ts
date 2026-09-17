import type { Animat, GraphClip, GraphType, Manifest, Mood } from '@vpet/shared'
import { PET_BASE, pick, resolveClips } from './manifest'

export interface PlayTarget {
  type: GraphType
  /** 动画名字；缺省 = type（如 default/default） */
  name?: string
  mood?: Mood
}

/** 'step' = 由外部状态机编排的单段播放，见 playStep */
type Phase = 'start' | 'loop' | 'end' | 'step'

/** 解码后的帧缓存上限。ImageBitmap 按 RGBA8 估算：size×size×4 字节/帧 */
const MAX_CACHED_BYTES = 192 * 1024 * 1024

/**
 * Canvas 2D 帧动画播放器。
 *
 * 三段式（docs/05 §3）：play() → start（若有）→ loop（随机变体，循环到 stop()）→ end（若有）→ onIdle。
 * 时间推进用 rAF + 累计时间，按每帧自带的 ms 走，不用 setInterval。
 * 帧解码用 createImageBitmap，按 clip 预解码并做 LRU 缓存。
 */
export class AnimationPlayer {
  private readonly ctx: CanvasRenderingContext2D
  private readonly cache = new Map<string, ImageBitmap[]>()
  private cachedBytes = 0

  private stepDone: (() => void) | null = null
  private target: Required<PlayTarget> | null = null
  private phase: Phase = 'loop'
  private clip: GraphClip | null = null
  private frames: ImageBitmap[] = []
  private frameIdx = 0
  private elapsed = 0
  private lastT = 0
  private stopping = false
  private raf = 0
  private generation = 0
  private destroyed = false

  /** loop 段自然结束、且已被 stop() 后回调（end 段播完） */
  onIdle: (() => void) | null = null

  /** 每画一帧回调一次，用来更新穿透判定的 alpha 掩码 */
  onFrame: ((bmp: ImageBitmap) => void) | null = null

  constructor(
    canvas: HTMLCanvasElement,
    private readonly manifest: Manifest,
  ) {
    const ctx = canvas.getContext('2d', { alpha: true })
    if (!ctx) throw new Error('无法创建 2D 上下文')
    this.ctx = ctx
    const dpr = window.devicePixelRatio || 1
    canvas.width = manifest.size * dpr
    canvas.height = manifest.size * dpr
    canvas.style.width = `${manifest.size}px`
    canvas.style.height = `${manifest.size}px`
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  }

  /** 播放一个目标动画；若当前有动画会直接切换（不等 end 段） */
  async play(t: PlayTarget): Promise<void> {
    const gen = ++this.generation
    this.target = { type: t.type, name: t.name ?? t.type, mood: t.mood ?? 'nomal' }
    this.stopping = false
    this.stepDone = null

    const start = this.resolve('start')
    const loop = this.resolve('loop')
    const single = this.resolve('single')
    const first = start.length ? { phase: 'start' as Phase, clip: pick(start) } : loop.length ? { phase: 'loop' as Phase, clip: pick(loop) } : single.length ? { phase: 'loop' as Phase, clip: pick(single) } : null
    if (!first) {
      console.warn(`[AnimationPlayer] 没有动画：${this.target.type}/${this.target.name}/${this.target.mood}`)
      this.onIdle?.()
      return
    }
    await this.switchTo(first.clip, first.phase, gen)
    // 预热下一段
    void this.preload(loop.length ? loop : single)
  }

  /** 播放一次：start → loop 只跑一遍 → end → onIdle */
  async playOnce(t: PlayTarget): Promise<void> {
    await this.play(t)
    this.stopping = true
  }

  /**
   * 只播一个段落，结束后回调，不自动接后续段落——供外部状态机逐段编排。
   * 提起就是这么拼的：raised_dynamic×3 → raised_static start → loop → end
   * （原版 MainDisplay.DisplayRaising 的 rasetype 递归，legacy MainDisplay.cs:370-416）。
   */
  async playStep(t: PlayTarget, animat: Animat, onDone?: () => void): Promise<void> {
    const gen = ++this.generation
    this.target = { type: t.type, name: t.name ?? t.type, mood: t.mood ?? 'nomal' }
    this.stopping = false
    const clips = this.resolve(animat)
    if (!clips.length) {
      this.stepDone = null
      onDone?.()
      return
    }
    this.stepDone = onDone ?? null
    await this.switchTo(pick(clips), 'step', gen)
  }

  /** 请求结束：当前 loop 跑完后进入 end 段，播完触发 onIdle */
  stop(): void {
    this.stopping = true
  }

  /** 帧缓存占用，用来盯住内存预算 */
  stats(): { clips: number; bytes: number } {
    return { clips: this.cache.size, bytes: this.cachedBytes }
  }

  destroy(): void {
    this.destroyed = true
    cancelAnimationFrame(this.raf)
    for (const frames of this.cache.values()) frames.forEach((b) => b.close())
    this.cache.clear()
    this.cachedBytes = 0
  }

  /* ------------------------------------------------------------ */

  private resolve(animat: 'start' | 'loop' | 'end' | 'single'): GraphClip[] {
    if (!this.target) return []
    return resolveClips(this.manifest, this.target.type, this.target.name, this.target.mood, animat)
  }

  private async switchTo(clip: GraphClip, phase: Phase, gen: number): Promise<void> {
    const frames = await this.decode(clip)
    if (gen !== this.generation || this.destroyed) return
    this.clip = clip
    this.phase = phase
    this.frames = frames
    this.frameIdx = 0
    this.elapsed = 0
    this.lastT = performance.now()
    this.draw()
    cancelAnimationFrame(this.raf)
    this.raf = requestAnimationFrame(this.tick)
  }

  private tick = (now: number): void => {
    if (this.destroyed || !this.clip) return
    const dt = Math.min(now - this.lastT, 250) // 窗口被隐藏后恢复时不要一次跳太多帧
    this.lastT = now
    this.elapsed += dt

    let advanced = false
    while (this.clip && this.elapsed >= this.clip.frames[this.frameIdx].ms) {
      this.elapsed -= this.clip.frames[this.frameIdx].ms
      this.frameIdx++
      advanced = true
      if (this.frameIdx >= this.clip.frames.length) {
        this.frameIdx = 0
        void this.onClipEnd()
        return
      }
    }
    if (advanced) this.draw()
    this.raf = requestAnimationFrame(this.tick)
  }

  private async onClipEnd(): Promise<void> {
    const gen = this.generation
    if (this.phase === 'step') {
      const done = this.stepDone
      this.stepDone = null
      this.clip = null
      if (gen === this.generation) done?.()
      return
    }
    if (this.phase === 'start') {
      const loop = this.resolve('loop')
      const single = this.resolve('single')
      const next = loop.length ? loop : single
      if (next.length) return this.switchTo(pick(next), 'loop', gen)
      return this.finish(gen)
    }
    if (this.phase === 'loop') {
      if (!this.stopping) {
        // 每一轮随机换一个变体（原版 Default 的行为）
        const loop = this.resolve('loop')
        const single = this.resolve('single')
        const next = loop.length ? loop : single
        return this.switchTo(next.length ? pick(next) : this.clip!, 'loop', gen)
      }
      return this.finish(gen)
    }
    // end 段播完
    this.clip = null
    if (gen === this.generation) this.onIdle?.()
  }

  private async finish(gen: number): Promise<void> {
    const end = this.resolve('end')
    if (end.length) return this.switchTo(pick(end), 'end', gen)
    this.clip = null
    if (gen === this.generation) this.onIdle?.()
  }

  private draw(): void {
    const bmp = this.frames[this.frameIdx]
    if (!bmp) return
    const s = this.manifest.size
    this.ctx.clearRect(0, 0, s, s)
    this.ctx.drawImage(bmp, 0, 0, s, s)
    this.onFrame?.(bmp)
  }

  private async decode(clip: GraphClip): Promise<ImageBitmap[]> {
    const hit = this.cache.get(clip.id)
    if (hit) {
      // LRU：重新插入到末尾
      this.cache.delete(clip.id)
      this.cache.set(clip.id, hit)
      return hit
    }
    const frames = await Promise.all(
      clip.frames.map(async (f) => {
        const res = await fetch(`${PET_BASE}/${f.src}`)
        if (!res.ok) throw new Error(`帧加载失败 ${f.src} (${res.status})`)
        return createImageBitmap(await res.blob())
      }),
    )
    if (this.destroyed) {
      frames.forEach((b) => b.close())
      return frames
    }
    this.cache.set(clip.id, frames)
    this.cachedBytes += this.bytesOf(frames.length)
    // 刚放进来的和正在播的都是最近使用的，不会排在队首，所以 break 只是兜底
    while (this.cachedBytes > MAX_CACHED_BYTES && this.cache.size > 1) {
      const [oldestId, oldest] = this.cache.entries().next().value as [string, ImageBitmap[]]
      if (oldestId === this.clip?.id || oldestId === clip.id) break
      oldest.forEach((b) => b.close())
      this.cache.delete(oldestId)
      this.cachedBytes -= this.bytesOf(oldest.length)
    }
    return frames
  }

  private bytesOf(frameCount: number): number {
    return frameCount * this.manifest.size * this.manifest.size * 4
  }

  private async preload(clips: GraphClip[]): Promise<void> {
    for (const c of clips.slice(0, 3)) {
      if (this.destroyed) return
      await this.decode(c).catch(() => undefined)
    }
  }
}
