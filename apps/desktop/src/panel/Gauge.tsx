/** 低于这个比例就变红，提示她快撑不住了 */
const LOW = 0.25

/**
 * `accent` 用来给「低了也不算告急」的量换个配色——好感度低只是生分，
 * 不是撑不住了，不该和饱腹见底一个颜色
 */
export function Gauge({ label, value, accent }: { label: string; value: number; accent?: string }) {
  const pct = Math.max(0, Math.min(100, value))
  const color = accent ?? (pct / 100 < LOW ? '#e06c75' : '#7cc47f')
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 8 }}>
      <span style={{ width: 40, color: '#8a8a93', fontSize: 13 }}>{label}</span>
      <div style={{ flex: 1, height: 8, borderRadius: 4, background: '#2c2c32', overflow: 'hidden' }}>
        <div style={{ width: `${pct}%`, height: '100%', background: color, transition: 'width .3s' }} />
      </div>
      <span style={{ width: 34, textAlign: 'right', fontVariantNumeric: 'tabular-nums', fontSize: 13 }}>
        {pct.toFixed(0)}
      </span>
    </div>
  )
}
