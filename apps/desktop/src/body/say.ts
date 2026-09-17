/** 气泡说完多久自动收起：按字数算，至少 3s、最多 15s */
export const hideDelayMs = (text: string) => Math.min(15_000, Math.max(3_000, text.length * 160))
