import { useEffect, useState } from 'react'
import type { PetState } from '@vpet/shared'
import { invokeCore, IS_TAURI } from '../body/ipc'
import { subscribePetState } from '../body/petState'
import { Gauge } from './Gauge'

/** 面板刷新节奏。Core 只在换动作时推事件，数值得自己拉 */
const REFRESH_MS = 1000

export function Panel() {
  const [state, setState] = useState<PetState | null>(null)
  const [version, setVersion] = useState('')
  const [gift, setGift] = useState<string | null>(null)

  useEffect(() => {
    void invokeCore<string>('app_version').then((v) => v && setVersion(v))
    const pull = () => void invokeCore<PetState>('get_pet_state').then((s) => s && setState(s))
    pull()
    const timer = window.setInterval(pull, REFRESH_MS)
    const stop = subscribePetState(setState)
    return () => {
      window.clearInterval(timer)
      stop()
    }
  }, [])

  const patch = (p: Record<string, number>) => void invokeCore('debug_patch_pet_state', p)

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
