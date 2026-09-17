import { useEffect, useState } from 'react'
import type { PetState } from '@vpet/shared'
import { invokeCore, IS_TAURI } from '../body/ipc'
import { subscribePetState } from '../body/petState'
import { Gauge } from './Gauge'

/** 面板刷新节奏。Core 只在换动作时推事件，数值得自己拉 */
const REFRESH_MS = 1000

interface AuditRow {
  at: number
  tool: string
  origin: string
  decision: 'allow' | 'ask' | 'deny'
  ok: boolean
  input: string
  summary: string
}

const DECISION_COLOR: Record<AuditRow['decision'], string> = {
  allow: '#7cc47f',
  ask: '#e0a458',
  deny: '#e06c75',
}

interface Pomo {
  phase: 'focus' | 'shortbreak' | 'longbreak'
  remainingSec: number
  completed: number
}

const PHASE_LABEL: Record<Pomo['phase'], string> = {
  focus: '专注',
  shortbreak: '短休',
  longbreak: '长休',
}

const mmss = (sec: number) => {
  const s = Math.max(0, Math.round(sec))
  return `${String(Math.floor(s / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`
}

interface TimerRow {
  id: string
  label: string
  dueAt: number
  repeatMs: number | null
}

/** 还剩多久。已经过点了就显示「就绪」——下一拍心跳会把它响掉 */
function remaining(dueAt: number): string {
  const left = Math.round((dueAt - Date.now()) / 1000)
  if (left <= 0) return '就绪'
  if (left < 60) return `${left}s`
  const m = Math.floor(left / 60)
  return m < 60 ? `${m}m${left % 60}s` : `${Math.floor(m / 60)}h${m % 60}m`
}

export function Panel() {
  const [state, setState] = useState<PetState | null>(null)
  const [version, setVersion] = useState('')
  const [gift, setGift] = useState<string | null>(null)
  const [timers, setTimers] = useState<TimerRow[]>([])
  const [pomo, setPomo] = useState<Pomo | null>(null)
  const [audit, setAudit] = useState<AuditRow[]>([])

  useEffect(() => {
    void invokeCore<string>('app_version').then((v) => v && setVersion(v))
    const pull = () => {
      void invokeCore<PetState>('get_pet_state').then((s) => s && setState(s))
      void invokeCore<TimerRow[]>('list_timers').then((t) => t && setTimers(t))
      void invokeCore<Pomo | null>('get_pomodoro').then(setPomo)
      void invokeCore<AuditRow[]>('recent_audit', { limit: 12 }).then((a) => a && setAudit(a))
    }
    pull()
    const timer = window.setInterval(pull, REFRESH_MS)
    const stop = subscribePetState(setState)
    return () => {
      window.clearInterval(timer)
      stop()
    }
  }, [])

  const patch = (p: Record<string, number>) => void invokeCore('debug_patch_pet_state', p)
  const pullTimers = () => void invokeCore<TimerRow[]>('list_timers').then((t) => t && setTimers(t))
  const addTimer = (duration: string, label: string, repeat = false) =>
    void invokeCore('create_timer', { duration, label, repeat }).then(pullTimers)
  const callTool = (name: string, input: Record<string, unknown>) =>
    void invokeCore('run_tool', {
      call: { callId: `panel-${Date.now()}`, name, input, origin: 'user' },
    }).then(() => {
      pullTimers()
      void invokeCore<AuditRow[]>('recent_audit', { limit: 12 }).then((a) => a && setAudit(a))
    })

  return (
    <div style={{ maxWidth: 720, margin: '0 auto', padding: '24px 20px 40px' }}>
      <header style={{ display: 'flex', alignItems: 'baseline', gap: 10, marginBottom: 20 }}>
        <h1 style={{ margin: 0, fontSize: 20 }}>VPet 面板</h1>
        <span style={{ color: '#8a8a93', fontSize: 13 }}>{version && `v${version}`}</span>
        {!IS_TAURI && (
          <span style={{ marginLeft: 'auto', color: '#e0a458', fontSize: 13 }}>
            浏览器预览：连不上 Core，下面是空的
          </span>
        )}
      </header>

      {!state ? (
        <p style={{ color: '#8a8a93' }}>还没拿到状态…</p>
      ) : (
        <>
          <Card title="此刻">
            <div style={{ display: 'flex', alignItems: 'baseline', gap: 12, flexWrap: 'wrap' }}>
              <strong style={{ fontSize: 22 }}>{state.action?.name ?? '—'}</strong>
              {state.action?.reason && (
                <span style={{ color: '#8ab4f8' }}>因为{state.action.reason}</span>
              )}
              {state.action?.food && (
                <span style={{ color: '#a8d5a2' }}>· {state.action.food.name}</span>
              )}
            </div>
            <div style={{ color: '#8a8a93', fontSize: 13, marginTop: 6 }}>
              活动 {state.activity} · 心情 {state.mood}
            </div>
          </Card>

          <Card title="身上的数值">
            <Gauge label="体力" value={state.strength} />
            <Gauge label="心情" value={state.feeling} />
            <Gauge label="饱腹" value={state.hunger} />
            <Gauge label="口渴" value={state.thirst} />
            <div style={{ display: 'flex', gap: 24, marginTop: 14, fontSize: 15 }}>
              <span>💰 {state.money.toFixed(1)}</span>
              <span>⭐ Lv{state.level}</span>
              <span style={{ color: '#8a8a93' }}>经验 {state.exp.toFixed(0)}</span>
            </div>
          </Card>

          <Card title="调试">
            <p style={{ margin: '0 0 12px', color: '#8a8a93', fontSize: 13 }}>
              直接改数值，看她会不会按预期换一件事做。数值怎么演变见 <code>actions.toml</code>。
            </p>
            <Row>
              <Btn onClick={() => patch({ hunger: 10 })}>饿到 10</Btn>
              <Btn onClick={() => patch({ thirst: 10 })}>渴到 10</Btn>
              <Btn onClick={() => patch({ strength: 10 })}>累到 10</Btn>
              <Btn onClick={() => patch({ feeling: 10 })}>心情 10</Btn>
            </Row>
            <Row>
              <Btn onClick={() => patch({ money: 0 })}>钱清零</Btn>
              <Btn onClick={() => patch({ money: 5000 })}>给她 5000</Btn>
              <Btn
                onClick={() => void invokeCore<string>('give_gift').then((n) => setGift(n ?? '（没货架）'))}
              >
                送个礼物
              </Btn>
              <Btn
                onClick={() =>
                  patch({ strength: 100, feeling: 60, hunger: 100, thirst: 100 })
                }
              >
                全部复原
              </Btn>
            </Row>
            {gift && <p style={{ color: '#a8d5a2', marginBottom: 0 }}>送出了：{gift}</p>}
          </Card>

          <Card title="工具调用">
            <p style={{ margin: '0 0 12px', color: '#8a8a93', fontSize: 13 }}>
              Brain 只能通过这条路动这个系统：过权限门 → 执行 → 落审计。下面的按钮走的是同一条路。
            </p>
            <Row>
              <Btn onClick={() => callTool('create_timer', { duration: '10s', label: '十秒到了' })}>
                run_tool(create_timer 10s)
              </Btn>
              <Btn onClick={() => callTool('get_pet_state', {})}>run_tool(get_pet_state)</Btn>
              <Btn onClick={() => callTool('set_permission', { scope: 'kb.read', decision: 'allow' })}>
                run_tool(set_permission) · 会被拦
              </Btn>
            </Row>
            {audit.length === 0 ? (
              <p style={{ color: '#8a8a93', margin: 0, fontSize: 13 }}>还没有调用记录</p>
            ) : (
              <ul style={{ listStyle: 'none', padding: 0, margin: 0, fontSize: 13 }}>
                {audit.map((a, i) => (
                  <li
                    key={`${a.at}-${i}`}
                    style={{ display: 'flex', gap: 10, padding: '5px 0', borderTop: '1px solid #2c2c32' }}
                  >
                    <span style={{ color: '#8a8a93', fontVariantNumeric: 'tabular-nums' }}>
                      {new Date(a.at).toLocaleTimeString('zh-CN', { hour12: false })}
                    </span>
                    <span style={{ color: DECISION_COLOR[a.decision], width: 40 }}>{a.decision}</span>
                    <span style={{ width: 120 }}>{a.tool}</span>
                    <span style={{ flex: 1, color: '#8a8a93', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {a.summary}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </Card>

          <Card title="番茄钟">
            {pomo ? (
              <>
                <div style={{ display: 'flex', alignItems: 'baseline', gap: 14, marginBottom: 12 }}>
                  <strong style={{ fontSize: 30, fontVariantNumeric: 'tabular-nums' }}>
                    {mmss(pomo.remainingSec)}
                  </strong>
                  <span style={{ color: pomo.phase === 'focus' ? '#8ab4f8' : '#a8d5a2' }}>
                    {PHASE_LABEL[pomo.phase]}
                  </span>
                  <span style={{ color: '#8a8a93', fontSize: 13 }}>已完成 {pomo.completed} 个</span>
                </div>
                <Btn onClick={() => void invokeCore('stop_pomodoro').then(() => setPomo(null))}>停止</Btn>
              </>
            ) : (
              <>
                <p style={{ margin: '0 0 12px', color: '#8a8a93', fontSize: 13 }}>
                  25 / 5，四个一轮转长休。跑着的时候她会一直干活，只有饿到不行才会去吃。
                </p>
                <Btn onClick={() => void invokeCore<Pomo>('start_pomodoro').then((p) => p && setPomo(p))}>
                  开始专注
                </Btn>
              </>
            )}
          </Card>

          <Card title="计时器">
            <p style={{ margin: '0 0 12px', color: '#8a8a93', fontSize: 13 }}>
              到点她会说一句。杀掉进程重启，没到期的还在。
            </p>
            <Row>
              <Btn onClick={() => addTimer('10s', '十秒到了')}>10 秒后</Btn>
              <Btn onClick={() => addTimer('25m', '该休息了')}>25 分钟后</Btn>
              <Btn onClick={() => addTimer('1m', '每分钟提醒', true)}>每分钟</Btn>
            </Row>
            {timers.length === 0 ? (
              <p style={{ color: '#8a8a93', margin: 0, fontSize: 13 }}>还没有计时器</p>
            ) : (
              <ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
                {timers.map((t) => (
                  <li
                    key={t.id}
                    style={{
                      display: 'flex', alignItems: 'center', gap: 10,
                      padding: '6px 0', borderTop: '1px solid #2c2c32',
                    }}
                  >
                    <span style={{ flex: 1 }}>
                      {t.label}
                      {t.repeatMs != null && <span style={{ color: '#8a8a93' }}> · 循环</span>}
                    </span>
                    <span style={{ color: '#8a8a93', fontVariantNumeric: 'tabular-nums', fontSize: 13 }}>
                      {remaining(t.dueAt)}
                    </span>
                    <Btn onClick={() => void invokeCore('cancel_timer', { id: t.id }).then(pullTimers)}>
                      取消
                    </Btn>
                  </li>
                ))}
              </ul>
            )}
          </Card>
        </>
      )}
    </div>
  )
}

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section
      style={{
        background: '#1f1f23', border: '1px solid #2c2c32', borderRadius: 12,
        padding: '16px 18px', marginBottom: 16,
      }}
    >
      <h2 style={{ margin: '0 0 12px', fontSize: 13, color: '#8a8a93', fontWeight: 600 }}>{title}</h2>
      {children}
    </section>
  )
}

const Row = ({ children }: { children: React.ReactNode }) => (
  <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', marginBottom: 10 }}>{children}</div>
)

function Btn({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      style={{
        padding: '7px 12px', borderRadius: 8, cursor: 'pointer',
        border: '1px solid #3a3a42', background: '#26262b', color: '#e8e8ea',
        font: '13px system-ui, sans-serif',
      }}
    >
      {children}
    </button>
  )
}
