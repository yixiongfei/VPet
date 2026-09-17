import { PET_LOGICAL_SIZE, type Animat, type FoodItem, type FoodKeyframe, type GraphClip, type GraphType, type LayeredClip, type Manifest, type Mood } from '@vpet/shared'
import { PET_BASE, byId, pick, pickFood, resolveClips, resolveLayered } from './manifest'

/**
 * 一条图层轨道。多层共用一个时钟（`elapsed`）：前后层帧数不同但总时长相同，
 * 各自按累计时间表映射到自己的帧，所以不会互相漂移。
 */
interface Track {
  clip: GraphClip
  frames: ImageBitmap[]
  /** 每帧的累计结束时间，用来由时间反查帧号 */
  cum: number[]
  idx: number
}

export interface PlayTarget {
  type: GraphType
  /** 动画名字；缺省 = type（如 default/default） */
  name?: string
  mood?: Mood
  /** 夹心动画中间那层用哪样食物。Core 挑好了就指定，没指定才随机 */
  foodId?: string
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
  private layered: LayeredClip | null = null
  /** 夹心中间那层：食物精灵 + 它自己的时间轴 */
  private food: { item: FoodItem; bmp: ImageBitmap; keys: FoodKeyframe[]; cum: number[] } | null = null
  private readonly foodCache = new Map<string, ImageBitmap>()
  /** Core 指定的食物 id；没指定就随机挑 */
  private foodId: string | undefined
  /** foodId 单独放在 this.foodId，不进 target */
  private target: Required<Omit<PlayTarget, 'foodId'>> | null = null
  private phase: Phase = 'loop'
  /** 绘制顺序：后层 → 前层。tracks[0] 是主轨，它播完就算这一段播完 */
  private tracks: Track[] = []
  private playing = false
  private elapsed = 0
  private lastT = 0
  private stopping = false
  private raf = 0
  private generation = 0
  private destroyed = false

  /** loop 段自然结束、且已被 stop() 后回调（end 段播完） */
  onIdle: (() => void) | null = null

  /** 每画完一帧（所有图层合成后）回调一次，用来更新穿透判定的 alpha 掩码 */
  onFrame: ((canvas: HTMLCanvasElement) => void) | null = null

  private readonly canvas: HTMLCanvasElement

  constructor(
    canvas: HTMLCanvasElement,
    private readonly manifest: Manifest,
  ) {
    const ctx = canvas.getContext('2d', { alpha: true })
    if (!ctx) throw new Error('无法创建 2D 上下文')
    this.ctx = ctx
    this.canvas = canvas
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
    this.layered = null
    this.food = null
    this.foodId = t.foodId

    // 夹心动画（吃 / 喝 / 收礼）走两层轨道
    const layered = resolveLayered(this.manifest, this.target.type, this.target.name, this.target.mood)
    if (layered) return this.switchToLayered(layered, 'loop', gen)

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
    for (const b of this.foodCache.values()) b.close()
    this.foodCache.clear()
  }

  /* ------------------------------------------------------------ */

  private resolve(animat: 'start' | 'loop' | 'end' | 'single'): GraphClip[] {
    if (!this.target) return []
    return resolveClips(this.manifest, this.target.type, this.target.name, this.target.mood, animat)
  }

  private async switchTo(clip: GraphClip, phase: Phase, gen: number): Promise<void> {
    const track = await this.loadTrack(clip)
    if (gen !== this.generation || this.destroyed) return
    this.begin([track], phase)
  }

  /** 夹心动画：后层 → 前层两条轨道同时跑（中间的食物精灵在 drawFood 里） */
  private async switchToLayered(l: LayeredClip, phase: Phase, gen: number): Promise<void> {
    const back = byId(this.manifest, l.back)
    const front = byId(this.manifest, l.front)
    if (!back || !front) {
      console.warn(`[AnimationPlayer] 夹心动画缺层：${l.id}`)
      return this.finish(gen)
    }
    // Core 挑好了就用它的，否则自己随机（Core 还没拿到目录时会这样）
    const item = (this.foodId && this.manifest.food.find((f) => f.id === this.foodId)) || pickFood(this.manifest, l.name)
    const [tracks, bmp] = await Promise.all([
      Promise.all([this.loadTrack(back), this.loadTrack(front)]),
      item ? this.loadFoodImage(item) : Promise.resolve(null),
    ])
    if (gen !== this.generation || this.destroyed) return
    this.layered = l
    // 食物有自己的时间轴（和前后层的总时长不一定完全相等），超出就停在最后一段
    let acc = 0
    this.food = bmp && item ? { item, bmp, keys: l.food, cum: l.food.map((k) => (acc += k.ms)) } : null
    this.begin(tracks, phase)
  }

  private async loadFoodImage(item: FoodItem): Promise<ImageBitmap | null> {
    const hit = this.foodCache.get(item.id)
    if (hit) return hit
    try {
      const res = await fetch(`${PET_BASE}/${item.src}`)
      if (!res.ok) throw new Error(String(res.status))
      const bmp = await createImageBitmap(await res.blob())
      this.foodCache.set(item.id, bmp)
      return bmp
    } catch (e) {
      console.warn(`[AnimationPlayer] 食物图加载失败 ${item.name}`, e)
      return null
    }
  }

  /**
   * 夹心中间那层。原版把图塞进一个 Width×Width 的方盒里（`Height = Width`）、
   * 按比例内接，再整体旋转（FoodAnimation.Animation.Run）。
   */
  private drawFood(): void {
    const f = this.food
    if (!f) return
    let i = 0
    while (i < f.cum.length - 1 && this.elapsed >= f.cum[i]) i++
    const k = f.keys[i]
    if (!k?.visible || k.width === undefined || k.x === undefined || k.y === undefined) return

    const unit = this.manifest.size / PET_LOGICAL_SIZE // 逻辑坐标 → 画布坐标
    const box = k.width * unit
    const scale = Math.min(box / f.bmp.width, box / f.bmp.height)
    const w = f.bmp.width * scale
    const h = f.bmp.height * scale
    const cx = (k.x + k.width / 2) * unit
    const cy = (k.y + k.width / 2) * unit

    this.ctx.save()
    this.ctx.globalAlpha = k.opacity ?? 1
    this.ctx.translate(cx, cy)
    if (k.rotate) this.ctx.rotate((k.rotate * Math.PI) / 180)
    this.ctx.drawImage(f.bmp, -w / 2, -h / 2, w, h)
    this.ctx.restore()
  }

  private begin(tracks: Track[], phase: Phase): void {
    this.tracks = tracks
    this.phase = phase
    this.playing = true
    this.elapsed = 0
    this.lastT = performance.now()
    this.draw()
    cancelAnimationFrame(this.raf)
    this.raf = requestAnimationFrame(this.tick)
  }

  private async loadTrack(clip: GraphClip): Promise<Track> {
    const frames = await this.decode(clip)
    let acc = 0
    const cum = clip.frames.map((f) => (acc += f.ms))
    return { clip, frames, cum, idx: 0 }
  }

  private tick = (now: number): void => {
    if (this.destroyed || !this.playing) return
    const dt = Math.min(now - this.lastT, 250) // 窗口被隐藏后恢复时不要一次跳太多帧
    this.lastT = now
    this.elapsed += dt

    // 主轨（tracks[0]）跑完就算这一段结束；夹心动画前后层总时长相同
    const total = this.tracks[0]?.cum.at(-1) ?? 0
    if (this.elapsed >= total) {
      this.playing = false
      void this.onClipEnd()
      return
    }

    let advanced = false
    for (const t of this.tracks) {
      // elapsed 单调递增，所以从当前帧往后找就够了；末尾停在最后一帧
      while (t.idx < t.cum.length - 1 && this.elapsed >= t.cum[t.idx]) {
        t.idx++
        advanced = true
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
        // 夹心动画只有一份，直接重播
        if (this.layered) return this.switchToLayered(this.layered, 'loop', gen)
        // 每一轮随机换一个变体（原版 Default 的行为）
        const loop = this.resolve('loop')
        const single = this.resolve('single')
        const next = loop.length ? loop : single
        return this.switchTo(next.length ? pick(next) : this.tracks[0].clip, 'loop', gen)
      }
      return this.finish(gen)
    }
    // end 段播完
    if (gen === this.generation) this.onIdle?.()
  }

  private async finish(gen: number): Promise<void> {
    const end = this.resolve('end')
    if (end.length) return this.switchTo(pick(end), 'end', gen)
    if (gen === this.generation) this.onIdle?.()
  }

  /** 后层 → 食物 → 前层，依次画上去（docs/05 §3 双图层） */
  private draw(): void {
    const s = this.manifest.size
    this.ctx.clearRect(0, 0, s, s)
    this.tracks.forEach((t, i) => {
      const bmp = t.frames[t.idx]
      if (bmp) this.ctx.drawImage(bmp, 0, 0, s, s)
      if (i === 0) this.drawFood() // 后层之后、前层（手）之前
    })
    this.onFrame?.(this.canvas)
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
      if (this.tracks.some((t) => t.clip.id === oldestId) || oldestId === clip.id) break
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
