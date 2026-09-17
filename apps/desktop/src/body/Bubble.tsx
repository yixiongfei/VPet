import { useEffect, useRef, useState } from 'react'

const MAX_LINES = 3
/** 与下面 font 简写里的行高保持一致 */
const LINE_HEIGHT = 1.6

export interface BubbleProps {
  name: string
  text: string
  /** 还在流式输出中：显示光标，且不自动收起 */
  streaming: boolean
  onClose: () => void
}

/**
 * 说话气泡。贴着窗口底部向上生长，盖在宠物身上——与原版
 * legacy/VPet-Simulator.Core/Display/MessageBar.xaml 一致（500×500 的层 + 底对齐）。
 * 超过 MAX_LINES 行折叠，点「展开」看全文。
 */
export function Bubble({ name, text, streaming, onClose }: BubbleProps) {
  const textRef = useRef<HTMLDivElement>(null)
  const [expanded, setExpanded] = useState(false)
  const [clamped, setClamped] = useState(false)

  // 用 maxHeight 裁而不是 -webkit-line-clamp：后者在布局层就截断了内容，
  // scrollHeight 会等于 clientHeight，溢出根本测不出来。
  useEffect(() => {
    const el = textRef.current
    if (!el || expanded) return
    setClamped(el.scrollHeight > el.clientHeight + 1)
  }, [text, expanded])

  // 换一段话就收回展开状态
  useEffect(() => setExpanded(false), [name])

  return (
    <div
      style={{
        padding: '12px 14px', borderRadius: 14,
        background: 'rgba(28,28,32,.88)', color: '#f4f4f5',
        font: '15px/1.6 system-ui, "Microsoft YaHei", sans-serif',
        boxShadow: '0 6px 24px rgba(0,0,0,.35)', backdropFilter: 'blur(6px)',
      }}
      onDoubleClick={onClose}
    >
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 8, marginBottom: 4 }}>
        <span style={{ fontWeight: 700, color: '#ffd9a0' }}>{name}</span>
        <button
          onClick={onClose}
          title="收起（Esc）"
          style={{
            marginLeft: 'auto', border: 0, background: 'transparent', cursor: 'pointer',
            color: '#9b9ba3', font: '14px system-ui, sans-serif', padding: '0 2px',
          }}
        >
          ×
        </button>
      </div>
      <div
        ref={textRef}
        style={
          expanded
            ? { maxHeight: 300, overflowY: 'auto', whiteSpace: 'pre-wrap' }
            : { maxHeight: `${MAX_LINES * LINE_HEIGHT}em`, overflow: 'hidden', whiteSpace: 'pre-wrap' }
        }
      >
        {text}
        {streaming && <span style={{ opacity: 0.55 }}>▍</span>}
      </div>
      {(clamped || expanded) && (
        <button
          onClick={() => setExpanded((v) => !v)}
          style={{
            marginTop: 4, border: 0, background: 'transparent', cursor: 'pointer',
            color: '#8ab4f8', font: '13px system-ui, sans-serif', padding: 0,
          }}
        >
          {expanded ? '收起' : '展开'}
        </button>
      )}
    </div>
  )
}
