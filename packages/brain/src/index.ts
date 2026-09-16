/**
 * @vpet/brain —— Agent 编排层（Phase 3 起实现）。
 *
 * 约束（docs/03-architecture.md §2）：
 *  - 零 React / DOM 依赖，可迁到 sidecar 或 Rust
 *  - 不持有计时、状态、权限逻辑；调用工具的唯一方式是 Core 的 `run_tool`
 *  - 被动运行：只被用户输入或 Core 的 `agent:trigger` 唤醒
 */
export interface ModelProvider {
  id: string
  stream(req: ChatRequest): AsyncIterable<ChatDelta>
}

export interface ChatRequest {
  model: string
  system: string
  messages: Message[]
  tools?: unknown[]
  temperature?: number
  maxTokens?: number
}

export type Message =
  | { role: 'user'; content: string }
  | { role: 'assistant'; content: string }
  | { role: 'tool'; callId: string; content: string }

export type ChatDelta =
  | { kind: 'text'; text: string }
  | { kind: 'tool_call'; callId: string; name: string; inputJson: string }
  | { kind: 'usage'; inputTokens: number; outputTokens: number }
  | { kind: 'done' }
