import { useEffect, useState } from 'react'
import type { PetState, Verdict } from '@vpet/shared'
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

interface MemoryRow {
  id: string
  content: string
  type: string
  importance: number
  confidence: number
  source: string
  pinned: boolean
  createdAt: number
  updatedAt: number
  lastAccessedAt: number
  accessCount: number
  expiresAt: number | null
  status: 'active' | 'archived' | 'deleted'
}

interface MemoryHealth {
  active: number
  archived: number
  deleted: number
  expired: number
  usedRatio: number
  avgImportance: number
  needsSweep: number
}

const MEM_TYPE_LABEL: Record<string, string> = {
  profile: '长期',
  preference: '偏好',
  habit: '习惯',
  temporary_context: '临时',
  relationship: '互动',
  commitment: '约定',
}

const MEM_SOURCE_LABEL: Record<string, string> = {
  user_explicit: '你说的',
  user_confirmed: '你确认过',
  inferred: '推断·未确认',
  system_event: '系统记的',
}

const MEM_STATUS_COLOR: Record<MemoryRow['status'], string> = {
  active: '#7cc47f',
  archived: '#8a8a93',
  deleted: '#e06c75',
}

const ymd = (ms: number) => new Date(ms).toISOString().slice(0, 10)

type EmbedState =
  | { state: 'disabled'; reason: string }
  | { state: 'loading' }
  | { state: 'ready'; name: string; dim: number }
  | { state: 'failed'; reason: string }

interface MemHit {
  item: MemoryRow
  similarity: number
  lexical: number
  dense: number | null
  score: number
}

interface BiasRow {
  tag: string
  weight: number
  halfLife: number
  age: number
}

const BIAS_LABEL: Record<string, string> = { work: '工作', study: '学习', play: '玩' }

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
  const [verdict, setVerdict] = useState<Verdict | null>(null)
  const [biases, setBiases] = useState<BiasRow[]>([])
  const [mems, setMems] = useState<MemoryRow[]>([])
  const [memHealth, setMemHealth] = useState<MemoryHealth | null>(null)
  const [memQuery, setMemQuery] = useState('')
  const [memCtx, setMemCtx] = useState('')
  const [memHits, setMemHits] = useState<MemHit[]>([])
  const [embed, setEmbed] = useState<EmbedState | null>(null)

  useEffect(() => {
    void invokeCore<string>('app_version').then((v) => v && setVersion(v))
    const pull = () => {
      void invokeCore<PetState>('get_pet_state').then((s) => s && setState(s))
      void invokeCore<TimerRow[]>('list_timers').then((t) => t && setTimers(t))
      void invokeCore<Pomo | null>('get_pomodoro').then(setPomo)
      void invokeCore<AuditRow[]>('recent_audit', { limit: 12 }).then((a) => a && setAudit(a))
      void invokeCore<BiasRow[]>('list_biases').then((b) => b && setBiases(b))
      void invokeCore<MemoryRow[]>('list_memories').then((m) => m && setMems(m))
      void invokeCore<MemoryHealth>('memory_health').then(setMemHealth)
      void invokeCore<EmbedState>('get_embed_state').then(setEmbed)
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
  const pullMems = () => {
    void invokeCore<MemoryRow[]>('list_memories').then((m) => m && setMems(m))
    void invokeCore<MemoryHealth>('memory_health').then(setMemHealth)
  }
  const memStatus = (id: string, status: string) =>
    void invokeCore('set_memory_status', { id, status }).then(pullMems)
  const memPin = (id: string, pinned: boolean) =>
    void invokeCore('pin_memory', { id, pinned }).then(pullMems)
  /** 预览「这个问题会带上哪些记忆」——排序的黑盒不给人看就成了玄学 */
  const previewCtx = () => {
    void invokeCore<string>('memory_context', { query: memQuery }).then((c) => {
      setMemCtx(c ?? '')
      pullMems()
    })
    // 同时把每条的字面分 / 语义分拉出来——看得见是哪一路召回的，才调得动
    void invokeCore<MemHit[]>('search_memory', { query: memQuery }).then((h) => setMemHits(h ?? []))
  }
  const rebuildIndex = () =>
    void invokeCore<number>('rebuild_embeddings').then(() => {
      pullMems()
      void invokeCore<EmbedState>('get_embed_state').then(setEmbed)
    })
  const bias = (tag: string, weight: number) =>
    void invokeCore<BiasRow[]>('set_bias', { tag, weight }).then((b) => b && setBiases(b))
  const unbias = (tag?: string) =>
    void invokeCore<BiasRow[]>('clear_bias', { tag }).then((b) => b && setBiases(b))
  /** 使唤她一次。返回的是判定结果，不是「已执行」 */
  const askFor = (target: string) =>
    void invokeCore<Verdict | null>('request_action', { target }).then(setVerdict)
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
            <Gauge label="好感" value={state.affection} accent="#c98bdb" />
            <div style={{ display: 'flex', gap: 24, marginTop: 14, fontSize: 15 }}>
              <span>💰 {state.money.toFixed(1)}</span>
              <span>⭐ Lv{state.level}</span>
              <span style={{ color: '#8a8a93' }}>经验 {state.exp.toFixed(0)}</span>
            </div>
          </Card>

          <Card title="使唤她">
            <p style={{ margin: '0 0 12px', color: '#8a8a93', fontSize: 13 }}>
              她<strong>不一定答应</strong>。服从概率由基线 + 好感 + 心情 −
              生理冲突 − 这件事本身的代价算出来，掷一次骰子决定。拒绝的理由来自冲突最大的那一项。
            </p>
            <Row>
              <Btn onClick={() => askFor('work')}>去工作</Btn>
              <Btn onClick={() => askFor('study')}>去学习</Btn>
              <Btn onClick={() => askFor('play')}>去玩</Btn>
              <Btn onClick={() => askFor('rest')}>去休息</Btn>
              <Btn onClick={() => askFor('eat')}>去吃饭</Btn>
            </Row>
            {verdict && (
              <p style={{ marginBottom: 0, color: verdict.obey ? '#7cc47f' : '#e0a458' }}>
                {verdict.obey ? '✓' : '✗'}「{verdict.action}」· {verdict.say}
                <span style={{ color: '#8a8a93', marginLeft: 8, fontVariantNumeric: 'tabular-nums' }}>
                  （这次有 {(verdict.p * 100).toFixed(0)}% 会听{verdict.refusal ? ` · ${verdict.refusal}` : ''}）
                </span>
              </p>
            )}
          </Card>

          <Card title="她记得什么">
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                marginBottom: 12,
                fontSize: 13,
                color: '#8a8a93',
              }}
            >
              <span>检索方式：</span>
              {embed?.state === 'ready' ? (
                <span style={{ color: '#7cc47f' }}>
                  语义 + 字面（{embed.name} · {embed.dim} 维）
                </span>
              ) : embed?.state === 'loading' ? (
                <span style={{ color: '#e0a458' }}>模型加载中，暂时只用字面</span>
              ) : (
                <span style={{ color: '#e0a458' }}>
                  只有字面
                  {embed && 'reason' in embed && `（${embed.reason}）`}
                </span>
              )}
              {embed?.state === 'ready' && <Btn onClick={rebuildIndex}>重建索引</Btn>}
            </div>
            {memHealth && (
              <p style={{ margin: '0 0 12px', color: '#8a8a93', fontSize: 13 }}>
                活跃 {memHealth.active} · 归档 {memHealth.archived} · 已删 {memHealth.deleted} ·
                被用过 {(memHealth.usedRatio * 100).toFixed(0)}% · 平均重要性{' '}
                {memHealth.avgImportance.toFixed(0)}
                {memHealth.needsSweep > 0 && (
                  <span style={{ color: '#e0a458' }}> · {memHealth.needsSweep} 条待打扫</span>
                )}
              </p>
            )}
            <div style={{ display: 'flex', gap: 8, marginBottom: 12 }}>
              <input
                value={memQuery}
                onChange={(e) => setMemQuery(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && previewCtx()}
                placeholder="试一个问题，看会带上哪些记忆"
                style={{
                  flex: 1,
                  background: '#2c2c32',
                  border: '1px solid #3a3a42',
                  borderRadius: 6,
                  color: '#e8e8ea',
                  padding: '6px 10px',
                  fontSize: 14,
                }}
              />
              <Btn onClick={previewCtx}>检索</Btn>
            </div>
            {memHits.length > 0 && (
              <ul style={{ listStyle: 'none', margin: '0 0 10px', padding: 0, fontSize: 12 }}>
                {memHits.map((h) => (
                  <li key={h.item.id} style={{ color: '#8a8a93', marginBottom: 3 }}>
                    <span style={{ fontVariantNumeric: 'tabular-nums' }}>
                      总分 {h.score.toFixed(3)} ← 字面 {h.lexical.toFixed(2)} ·{' '}
                      {h.dense == null ? '语义 —' : `语义 ${h.dense.toFixed(2)}`}
                    </span>
                    <span style={{ color: '#e8e8ea', marginLeft: 8 }}>{h.item.content}</span>
                  </li>
                ))}
              </ul>
            )}
            {memCtx && (
              <pre
                style={{
                  background: '#202026',
                  border: '1px solid #3a3a42',
                  borderRadius: 6,
                  padding: 10,
                  fontSize: 12,
                  color: '#a8d5a2',
                  whiteSpace: 'pre-wrap',
                  margin: '0 0 12px',
                }}
              >
                {memCtx}
              </pre>
            )}
            {mems.length === 0 ? (
              <p style={{ margin: 0, color: '#8a8a93', fontSize: 13 }}>
                她还什么都没记住。对着她说「记住：…」试试。
              </p>
            ) : (
              <ul style={{ listStyle: 'none', margin: 0, padding: 0 }}>
                {mems.map((m) => (
                  <li
                    key={m.id}
                    style={{
                      display: 'flex',
                      alignItems: 'baseline',
                      gap: 8,
                      fontSize: 13,
                      marginBottom: 8,
                      opacity: m.status === 'active' ? 1 : 0.5,
                    }}
                  >
                    <span style={{ color: MEM_STATUS_COLOR[m.status], fontSize: 11 }}>●</span>
                    <span style={{ color: '#8ab4f8', width: 36 }}>
                      {MEM_TYPE_LABEL[m.type] ?? m.type}
                    </span>
                    <span
                      style={{
                        flex: 1,
                        textDecoration: m.status === 'deleted' ? 'line-through' : 'none',
                      }}
                    >
                      {m.pinned && '📌 '}
                      {m.content}
                    </span>
                    <span style={{ color: '#8a8a93', fontSize: 11, whiteSpace: 'nowrap' }}>
                      {MEM_SOURCE_LABEL[m.source] ?? m.source} · 重{m.importance.toFixed(0)} · 用
                      {m.accessCount}
                      {m.expiresAt && ` · 至${ymd(m.expiresAt)}`}
                    </span>
                    {m.status === 'active' && (
                      <>
                        <Btn onClick={() => memPin(m.id, !m.pinned)}>{m.pinned ? '取消置顶' : '置顶'}</Btn>
                        <Btn onClick={() => memStatus(m.id, 'archived')}>归档</Btn>
                        <Btn onClick={() => memStatus(m.id, 'deleted')}>删除</Btn>
                      </>
                    )}
                    {m.status === 'archived' && <Btn onClick={() => memStatus(m.id, 'active')}>恢复</Btn>}
                  </li>
                ))}
              </ul>
            )}
          </Card>

          <Card title="长期倾向">
            <p style={{ margin: '0 0 12px', color: '#8a8a93', fontSize: 13 }}>
              「多工作一点」是<strong>持续的倾向</strong>，不是一次性的命令。它只在她自己决策时加一份权重，
              而且<strong>带半衰期</strong>（默认两小时）——随口一句话不该绑架她一辈子。
              吃喝睡不在可调之列：那是生理，调不了。
            </p>
            <Row>
              <Btn onClick={() => bias('work', 1)}>多工作</Btn>
              <Btn onClick={() => bias('study', 1)}>多学习</Btn>
              <Btn onClick={() => bias('play', -1)}>少玩点</Btn>
              <Btn onClick={() => unbias()}>全撤</Btn>
            </Row>
            {biases.length === 0 ? (
              <p style={{ margin: 0, color: '#8a8a93', fontSize: 13 }}>现在没有任何倾向</p>
            ) : (
              <ul style={{ listStyle: 'none', margin: 0, padding: 0 }}>
                {biases.map((b) => (
                  <li
                    key={b.tag}
                    style={{ display: 'flex', alignItems: 'center', gap: 10, fontSize: 14, marginBottom: 6 }}
                  >
                    <span style={{ width: 44 }}>{BIAS_LABEL[b.tag] ?? b.tag}</span>
                    <span
                      style={{
                        color: b.weight > 0 ? '#7cc47f' : '#e06c75',
                        fontVariantNumeric: 'tabular-nums',
                        width: 52,
                      }}
                    >
                      {b.weight > 0 ? '+' : ''}
                      {b.weight.toFixed(2)}
                    </span>
                    <span style={{ color: '#8a8a93', fontSize: 12 }}>
                      半衰期 {b.halfLife.toFixed(0)}m · 已过 {b.age.toFixed(0)}m
                    </span>
                    <Btn onClick={() => unbias(b.tag)}>撤</Btn>
                  </li>
                ))}
              </ul>
            )}
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
              <Btn onClick={() => patch({ affection: 0 })}>好感清零</Btn>
              <Btn onClick={() => patch({ affection: 100 })}>好感拉满</Btn>
              <Btn
                onClick={() => void invokeCore<string>('give_gift').then((n) => setGift(n ?? '（没货架）'))}
              >
                送个礼物
              </Btn>
              <Btn
                onClick={() =>
                  patch({ strength: 100, feeling: 60, hunger: 100, thirst: 100, affection: 50 })
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
