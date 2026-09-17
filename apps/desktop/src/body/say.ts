/** 气泡说完多久自动收起：按字数算，至少 3s、最多 15s */
export const hideDelayMs = (text: string) => Math.min(15_000, Math.max(3_000, text.length * 160))

/**
 * Phase 1 还没有 Brain（roadmap 3.2 ModelProvider / 3.3 AgentLoop），先用写死的回话
 * 把「流式 token → 气泡」这条链路跑通。Phase 3 换成 ModelProvider 的 token 流，
 * 这里的形状（异步生成器 yield 片段）不变，调用方不用改。
 */
export async function* cannedReply(userText: string): AsyncGenerator<string> {
  const reply = `我还没有大脑，接不上话——Phase 3 才会有。你刚说的是：「${userText}」`
  for (const ch of reply) {
    await new Promise((r) => setTimeout(r, 45))
    yield ch
  }
}
