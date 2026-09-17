import { useEffect, useRef, useState } from 'react'

export interface ChatInputProps {
  onSubmit: (text: string) => void
  onCancel: () => void
}

/** 气泡下方的输入框。Esc 收起，Enter 发送（docs/05 §4） */
export function ChatInput({ onSubmit, onCancel }: ChatInputProps) {
  const ref = useRef<HTMLInputElement>(null)
  const [value, setValue] = useState('')

  useEffect(() => ref.current?.focus(), [])

  return (
    <input
      ref={ref}
      value={value}
      placeholder="说点什么…（Esc 收起）"
      onChange={(e) => setValue(e.target.value)}
      onKeyDown={(e) => {
        e.stopPropagation()
        if (e.key === 'Escape') onCancel()
        else if (e.key === 'Enter' && value.trim()) {
          onSubmit(value.trim())
          setValue('')
        }
      }}
      style={{
        padding: '10px 14px', borderRadius: 12, border: '1px solid rgba(255,255,255,.18)',
        background: 'rgba(28,28,32,.92)', color: '#f4f4f5', outline: 'none',
        font: '15px system-ui, "Microsoft YaHei", sans-serif',
        boxShadow: '0 6px 24px rgba(0,0,0,.35)',
      }}
    />
  )
}
