export const IS_TAURI = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

let core: Promise<typeof import('@tauri-apps/api/core')> | null = null

/**
 * 调 Core 的命令。浏览器预览里没有 Tauri，返回 null 而不是抛——
 * 前端在没有 Core 的情况下也得能跑起来（动画、交互都不依赖它）。
 */
export async function invokeCore<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T | null> {
  if (!IS_TAURI) return null
  try {
    const { invoke } = await (core ??= import('@tauri-apps/api/core'))
    return await invoke<T>(cmd, args)
  } catch (e) {
    console.warn(`[VPet] ${cmd} 失败`, e)
    return null
  }
}
