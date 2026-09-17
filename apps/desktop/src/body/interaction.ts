import type { GraphType, Manifest, PetProfile, PetState } from '@vpet/shared'
import type { AnimationPlayer } from './AnimationPlayer'
import { namesFor, pick } from './manifest'
import { CLIP_FOR, DEFAULT_PET_STATE } from './petState'
import { moveWindowBy } from './petWindow'
import { clickZone, pressZone, type ClickZone } from './touch'

/** 按住多久算长按（原版 Setting.PressLength，默认 0.5s） */
const PRESS_MS = 500
/** 空闲多久随机播一个小动作 */
const IDLE_ACTION_EVERY_MS: [number, number] = [15_000, 40_000]
/** 提起后先挣扎几次再转静止（原版 rasetype 0→2，共 3 次 Raised_Dynamic） */
const STRUGGLE_TIMES = 3

export interface InteractionOpts {
  player: AnimationPlayer
  manifest: Manifest
  profile: PetProfile
  /** 交互发生时通知外部；Phase 2 起转给 Core 的状态机改体力/心情 */
  onTouch?: (zone: ClickZone | 'raise') => void
}

/**
 * 触摸交互状态机，移植自 legacy/VPet-Simulator.Core/Display/Main.xaml.cs 的鼠标处理：
 *
 *   短按（< PRESS_MS）命中头/身体 → 摸头 / 摸身体，三段式播完回当前活动
 *   长按（≥ PRESS_MS）命中提起区 → 挣扎×3 → 静止循环，窗口跟着光标走
 *   松手                          → 当前段播完 → 落地 → 回当前活动
 *
 * 判定用的是松手/长按那一刻的光标位置，不是按下的位置（与原版一致）。
 */
export class Interaction {
  /** 交互模式，和 PetState.activity 是两回事：这个说「手正在干什么」 */
  private mode: 'idle' | 'touching' | 'raised' = 'idle'
  private state: PetState = DEFAULT_PET_STATE
  private idleTimer = 0
  private pressTimer = 0
  private lastAt: { x: number; y: number } | null = null
  private struggles = 0
  private released = false
  private raiseName = 'raise'
  private followRaf = 0
  private pendingFollow: { dx: number; dy: number } | null = null
  private disposed = false

  constructor(private readonly o: InteractionOpts) {}

  /** 开始播当前活动对应的动画 + 空闲小动作循环 */
  start(): void {
    this.o.player.onIdle = () => {
      if (!this.disposed) this.toActivity()
    }
    this.toActivity()
  }

  /**
   * Core 推来新状态。正在摸 / 提起时不打断，等这段交互结束后
   * toActivity() 自然会用上新状态。
   */
  setState(s: PetState): void {
    if (this.disposed) return
    const changed = s.activity !== this.state.activity || s.mood !== this.state.mood
    this.state = s
    if (changed && this.mode === 'idle') this.toActivity()
  }

  dispose(): void {
    this.disposed = true
    window.clearTimeout(this.idleTimer)
    window.clearTimeout(this.pressTimer)
    cancelAnimationFrame(this.followRaf)
    this.o.player.onIdle = null
  }

  onPointerDown(x: number, y: number): void {
    if (this.disposed || this.mode === 'raised') return
    this.lastAt = { x, y }
    window.clearTimeout(this.idleTimer)
    window.clearTimeout(this.pressTimer)
    this.pressTimer = window.setTimeout(() => {
      this.pressTimer = 0
      const at = this.lastAt
      if (this.disposed || !at) return
      if (pressZone(this.o.profile, this.state.mood, at.x, at.y)) this.raise(at.x, at.y)
    }, PRESS_MS)
  }

  onPointerMove(x: number, y: number): void {
    if (this.disposed) return
    this.lastAt = { x, y }
    if (this.mode === 'raised') this.scheduleFollow(x, y)
  }

  onPointerUp(): void {
    if (this.disposed) return
    const wasShortPress = this.pressTimer !== 0
    window.clearTimeout(this.pressTimer)
    this.pressTimer = 0

    if (this.mode === 'raised') {
      this.released = true // 等当前段播完再落地
      return
    }
    if (!wasShortPress) return // 长按已经处理过了

    const at = this.lastAt
    const zone = at && clickZone(this.o.profile, at.x, at.y)
    if (zone) this.touch(zone)
    else this.scheduleIdleAction() // 点在空白处：恢复空闲计时
  }

  /* ------------------------------------------------------------ */

  /** 回到当前活动对应的循环动画 */
  private toActivity(): void {
    this.mode = 'idle'
    const { type, name } = CLIP_FOR[this.state.activity]
    void this.o.player.play({ type, name: this.nameFor(type, name), mood: this.state.mood })
    this.scheduleIdleAction()
  }

  /** 没指定名字时按心情随机挑一个（原版 GraphCore.FindName 的语义） */
  private nameFor(type: GraphType, explicit?: string): string | undefined {
    if (explicit) return explicit
    const names = namesFor(this.o.manifest, type, this.state.mood)
    return names.length ? pick(names) : undefined
  }

  /** 只有真正空闲时才插小动作——工作/睡觉时乱插会打断那个活动的循环 */
  private scheduleIdleAction(): void {
    window.clearTimeout(this.idleTimer)
    if (this.state.activity !== 'idle') return
    const names = namesFor(this.o.manifest, 'idel', this.state.mood)
    if (!names.length) return
    const [lo, hi] = IDLE_ACTION_EVERY_MS
    this.idleTimer = window.setTimeout(
      () => {
        if (this.disposed || this.mode !== 'idle') return
        void this.o.player.playOnce({ type: 'idel', name: pick(names), mood: this.state.mood })
      },
      lo + Math.random() * (hi - lo),
    )
  }

  private touch(zone: ClickZone): void {
    this.mode = 'touching'
    const mood = this.state.mood
    const type = zone === 'head' ? 'touch_head' : 'touch_body'
    void this.o.player.playOnce({ type, name: this.nameFor(type), mood })
    this.o.onTouch?.(zone)
  }

  private raise(x: number, y: number): void {
    this.mode = 'raised'
    this.released = false
    this.struggles = 0
    const mood = this.state.mood
    const names = namesFor(this.o.manifest, 'raised_static', mood)
    this.raiseName = names.length ? pick(names) : 'raise'
    this.o.onTouch?.('raise')
    // 先把窗口挪到「锚点贴着光标」（原版 DisplayRaised 的位置迁移）
    const anchor = this.o.profile.raisePoint[mood]
    void moveWindowBy(x - anchor.x, y - anchor.y)
    this.raiseStep()
  }

  /** 原版 MainDisplay.DisplayRaising 的 rasetype 递归 */
  private raiseStep = (): void => {
    if (this.disposed) return
    const mood = this.state.mood
    const name = this.raiseName
    if (this.released) {
      void this.o.player.playStep({ type: 'raised_static', name, mood }, 'end', () => this.toActivity())
      return
    }
    if (this.struggles < STRUGGLE_TIMES) {
      this.struggles++
      void this.o.player.playStep({ type: 'raised_dynamic', name, mood }, 'single', this.raiseStep)
      return
    }
    if (this.struggles === STRUGGLE_TIMES) {
      this.struggles++
      void this.o.player.playStep({ type: 'raised_static', name, mood }, 'start', this.raiseStep)
      return
    }
    void this.o.player.playStep({ type: 'raised_static', name, mood }, 'loop', this.raiseStep)
  }

  /** 提起时窗口跟随光标：光标在窗口内的位置会收敛到 raisePoint，所以偏移量自然趋于 0 */
  private scheduleFollow(x: number, y: number): void {
    const anchor = this.o.profile.raisePoint[this.state.mood]
    this.pendingFollow = { dx: x - anchor.x, dy: y - anchor.y }
    if (this.followRaf) return
    this.followRaf = requestAnimationFrame(() => {
      this.followRaf = 0
      const f = this.pendingFollow
      this.pendingFollow = null
      if (f && this.mode === 'raised' && !this.disposed) void moveWindowBy(f.dx, f.dy)
    })
  }
}
