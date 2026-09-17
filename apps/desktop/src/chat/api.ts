import { IS_TAURI } from '../body/ipc'

export interface Persona {
  name: string
  background: string
  appearance: string
  personality: string
  speakingStyle: string
}
export interface ChatSettings {
  model: string
  endpoint: string
  temperature: number
  persona: Persona
}
export interface ChatMessage {
  id: string
  role: 'user' | 'assistant'
  content: string
  createdAt: number
  status: 'complete' | 'cancelled' | 'error'
  rating: 'up' | 'down' | null
  correctedText: string | null
  source: 'model' | 'memory'
}
export interface ModelStatus { connected: boolean; models: string[]; error: string | null }
export interface DesktopSettings { size: number; alwaysOnTop: boolean }
export interface StreamEvent { requestId: string; delta: string; done: boolean; text?: string }

export const DEFAULT_CHAT_SETTINGS: ChatSettings = {
  model: 'qwen3.5:9b', endpoint: 'http://127.0.0.1:11434', temperature: 0.75,
  // 与 chat.rs 的 Persona::default 保持一致；真正生效的是 Core 里的那份，这里只是加载前的占位
  persona: {
    name: '萝莉斯',
    background: '住在用户桌面上的伙伴，陪伴日常生活、学习和工作，有自己的喜好与小脾气。',
    appearance: '与桌面立绘一致：银灰色长发、头顶一撮呆毛，金黄色的眼睛，穿着舒适可爱的日常装扮。',
    personality: '温柔、好奇、坦率，亲近但有边界。认真倾听，不一味附和。',
    speakingStyle: '用自然的简体中文聊天，通常两到四句话。像熟悉的朋友，不用客服套话；偶尔有轻巧的动作描写，不每句都撒娇。',
  },
}

/** Mutations must surface Core errors; a browser preview never pretends it saved. */
export async function invokeStrict<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!IS_TAURI) throw new Error('当前是浏览器预览，请在桌宠应用中使用这个功能。')
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<T>(command, args)
}
export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
