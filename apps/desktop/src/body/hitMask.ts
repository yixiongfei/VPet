import { PET_LOGICAL_SIZE } from '@vpet/shared'

/**
 * 命中掩码的分辨率。500 / 48 ≈ 10.4 逻辑像素一格——对「光标是不是落在宠物身上」
 * 这个判断足够了，而且格子粗一点反而更好点中（边缘有点宽容度）。
 * 48×48 = 2304 位 = 288 字节，按位打包后推给 Rust 很便宜。
 */
const N = 48
/** 低于这个 alpha 当作透明。抗锯齿的边缘像素 alpha 很低，不该算命中 */
const ALPHA_THRESHOLD = 16

const BYTES = Math.ceil((N * N) / 8)

/**
 * 从当前帧算出「哪些格子是不透明的」，供 Rust 侧判定光标是否压在宠物身上
 * （docs/05 §4 的 alpha 命中）。
 *
 * 做法是把 500×500 的帧缩到 48×48 再读回 alpha：readback 只有 ~9KB，
 * 比直接对原图 getImageData（1MB）便宜两个数量级。
 */
export class HitMask {
  private readonly ctx: OffscreenCanvasRenderingContext2D
  private readonly packed = new Uint8Array(BYTES)
  private readonly prev = new Uint8Array(BYTES)
  private hasPrev = false

  constructor() {
    const canvas = new OffscreenCanvas(N, N)
    const ctx = canvas.getContext('2d', { alpha: true, willReadFrequently: true })
    if (!ctx) throw new Error('无法创建掩码用的 2D 上下文')
    this.ctx = ctx
  }

  /** 用这一帧重算掩码；掩码没变化时返回 null，避免白推一次 IPC */
  update(bmp: ImageBitmap): Uint8Array | null {
    this.ctx.clearRect(0, 0, N, N)
    this.ctx.drawImage(bmp, 0, 0, N, N)
    const { data } = this.ctx.getImageData(0, 0, N, N)

    this.packed.fill(0)
    for (let i = 0; i < N * N; i++) {
      if (data[i * 4 + 3] > ALPHA_THRESHOLD) this.packed[i >> 3] |= 1 << (i & 7)
    }
    if (this.hasPrev && this.packed.every((b, i) => b === this.prev[i])) return null
    this.prev.set(this.packed)
    this.hasPrev = true
    return this.packed
  }

  /** 逻辑坐标（0–500）是否压在宠物身上。与 Rust 侧的查表必须同一套算法 */
  isOpaqueAt(x: number, y: number): boolean {
    const cell = cellIndex(x, y)
    return cell !== null && (this.packed[cell >> 3] & (1 << (cell & 7))) !== 0
  }

  /** 调试用：把掩码摊成 N×N 的 0/1，方便肉眼比对轮廓 */
  toGrid(): number[][] {
    return Array.from({ length: N }, (_, r) =>
      Array.from({ length: N }, (_, c) => {
        const i = r * N + c
        return (this.packed[i >> 3] & (1 << (i & 7))) !== 0 ? 1 : 0
      }),
    )
  }

  static readonly size = N
}

function cellIndex(x: number, y: number): number | null {
  const col = Math.floor((x / PET_LOGICAL_SIZE) * N)
  const row = Math.floor((y / PET_LOGICAL_SIZE) * N)
  if (col < 0 || col >= N || row < 0 || row >= N) return null
  return row * N + col
}
