import { useEffect, useRef, useState } from 'react'
import { IS_TAURI } from '../body/ipc'
import { DEFAULT_CHAT_SETTINGS, errorText, invokeStrict } from './api'
import type { ChatMessage, ChatSettings, ModelStatus, StreamEvent } from './api'
import { Icon } from './Icons'

const time = (at: number) => new Date(at).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })

export function Chat() {
  const [settings, setSettings] = useState<ChatSettings>(DEFAULT_CHAT_SETTINGS)
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [status, setStatus] = useState<ModelStatus | null>(null)
  const [text, setText] = useState('')
  const [busy, setBusy] = useState(false)
  const [cancelling, setCancelling] = useState(false)
  const [stream, setStream] = useState('')
  const [error, setError] = useState('')
  const [feedbackBusy, setFeedbackBusy] = useState<string | null>(null)
  const [editing, setEditing] = useState<ChatMessage | null>(null)
  const [correction, setCorrection] = useState('')
  const request = useRef<string | null>(null)
  const end = useRef<HTMLDivElement>(null)
  const input = useRef<HTMLTextAreaElement>(null)
  const nearBottom = useRef(true)

  useEffect(() => {
    if (!IS_TAURI) return
    let live = true
    const cleanups: Array<() => void> = []
    void Promise.all([
      invokeStrict<ChatSettings>('get_chat_settings'),
      invokeStrict<ChatMessage[]>('list_chat_messages'),
      invokeStrict<ModelStatus>('get_model_status'),
    ]).then(([config, history, model]) => {
      if (live) { setSettings(config); setMessages(history); setStatus(model) }
    }).catch(e => live && setError(errorText(e)))
    void import('@tauri-apps/api/event').then(async ({ listen }) => {
      const streamOff = await listen<StreamEvent>('chat-stream', ({ payload }) => {
        if (live && payload.requestId === request.current) setStream(s => s + payload.delta)
      })
      if (!live) streamOff(); else cleanups.push(streamOff)
      const settingsOff = await listen('chat:settings-changed', () => {
        void invokeStrict<ChatSettings>('get_chat_settings').then(s => live && setSettings(s))
        void invokeStrict<ModelStatus>('get_model_status').then(s => live && setStatus(s))
      })
      if (!live) settingsOff(); else cleanups.push(settingsOff)
    }).catch(e => live && setError(errorText(e)))
    return () => { live = false; cleanups.forEach(fn => fn()) }
  }, [])

  useEffect(() => {
    if (nearBottom.current) end.current?.scrollIntoView({ behavior: busy ? 'instant' : 'smooth', block: 'end' })
  }, [messages, stream, busy])

  const openSettings = async () => {
    if (!IS_TAURI) { window.location.href = '/panel.html'; return }
    try { await invokeStrict('open_settings_panel') } catch (e) { setError(errorText(e)) }
  }

  const send = async () => {
    const content = text.trim()
    if (!content || request.current) return
    if (!IS_TAURI) { setError('这是界面预览。请点击桌面上的人物，在应用里开始本地对话。'); return }
    const requestId = crypto.randomUUID()
    request.current = requestId
    nearBottom.current = true
    setBusy(true); setStream(''); setError(''); setText('')
    setMessages(old => [...old, { id: `pending-${requestId}`, role: 'user', content, createdAt: Date.now(), status: 'complete', rating: null, correctedText: null, source: 'model' }])
    try {
      const answer = await invokeStrict<ChatMessage>('send_chat_message', { text: content, requestId })
      setMessages(old => [...old, answer])
    } catch (e) {
      setError(errorText(e))
    } finally {
      try { setMessages(await invokeStrict<ChatMessage[]>('list_chat_messages')) }
      catch (e) { setError(errorText(e)) }
      request.current = null
      setBusy(false); setCancelling(false); setStream('')
      input.current?.focus()
      void invokeStrict<ModelStatus>('get_model_status').then(setStatus).catch(() => {})
    }
  }

  const cancel = async () => {
    if (!request.current || cancelling) return
    setCancelling(true)
    try { await invokeStrict('cancel_chat', { requestId: request.current }) }
    catch (e) { setError(errorText(e)); setCancelling(false) }
  }

  const rate = async (message: ChatMessage, rating: 'up' | 'down' | null, correctedText = message.correctedText) => {
    if (feedbackBusy) return
    setFeedbackBusy(message.id); setError('')
    try {
      const updated = await invokeStrict<ChatMessage>('rate_chat_message', { messageId: message.id, rating, correctedText })
      setMessages(old => old.map(m => m.id === updated.id ? updated : m))
      setEditing(null)
    } catch (e) { setError(errorText(e)) }
    finally { setFeedbackBusy(null) }
  }

  const connected = status?.connected && status.models.some(m => m === settings.model || m === `${settings.model}:latest`)

  return <main className="chat-app">
    <header className="chat-header">
      <div className="companion-identity">
        <div className="portrait portrait-small"><img src="/avatar.png" alt={`${settings.persona.name}的头像`} /></div>
        <div><h1>{settings.persona.name}<span className="identity-tag">桌面伙伴</span></h1>
          <p><span className={`status-dot ${connected ? 'is-online' : ''}`} />{!IS_TAURI ? '界面预览' : connected ? '本地陪伴 · 对话留在这台电脑' : status ? '本地模型待连接' : '正在连接本地模型'}</p>
        </div>
      </div>
      <button className="icon-button" title="个性与设置" aria-label="个性与设置" onClick={() => void openSettings()}><Icon name="settings" size={21} /></button>
    </header>

    {!IS_TAURI && <div className="preview-notice">浏览器预览 · 真实对话、记忆和设置在桌宠应用中启用</div>}

    <section className="chat-history" aria-label="对话记录" onScroll={e => {
      const area = e.currentTarget
      nearBottom.current = area.scrollHeight - area.scrollTop - area.clientHeight < 100
    }}>
      {messages.length === 0 && !busy ? <div className="welcome">
        <div className="welcome-art"><span className="welcome-orbit orbit-one" /><span className="welcome-orbit orbit-two" /><div className="portrait portrait-hero"><img src="/avatar.png" alt="" /></div><span className="little-spark"><Icon name="spark" size={23} /></span></div>
        <p className="eyebrow">A LITTLE COMPANY, EVERY DAY</p>
        <h2>给日常，留一点陪伴。</h2>
        <p className="welcome-copy">今天的小事、突然的想法，或是一个晚安。<br />从第一句话开始，慢慢熟悉彼此。</p>
        <div className="prompt-suggestions">
          {['今天想和你聊聊', '一起安排今天吧', '你记得我什么？'].map((suggestion, i) => <button key={suggestion} onClick={() => { setText(suggestion); input.current?.focus() }}><Icon name={i === 0 ? 'chat' : i === 1 ? 'leaf' : 'spark'} size={17} />{suggestion}<Icon name="arrow" size={15} /></button>)}
        </div>
        <p className="welcome-footnote">你可以在设置里，为她写下独一无二的个性。</p>
      </div> : <div className="message-list">
        <div className="conversation-label"><span />你们的日常<span /></div>
        {messages.map(message => <article className={`message message-${message.role} ${message.status === 'error' ? 'message-error' : ''}`} key={message.id}>
          {message.role === 'assistant' && <div className="portrait portrait-message"><img src="/avatar.png" alt="" /></div>}
          <div className="message-content">
            <div className="message-meta"><span>{message.role === 'assistant' ? settings.persona.name : '你'}</span><time dateTime={new Date(message.createdAt).toISOString()}>{time(message.createdAt)}</time></div>
            <div className="message-bubble">{message.content || (message.status === 'cancelled' ? '这次回答已停止。' : '这次没有生成回答。')}</div>
            {message.status !== 'complete' && <p className="message-state">{message.status === 'cancelled' ? '已停止生成' : '生成未完成'}</p>}
            {message.role === 'assistant' && message.status === 'complete' && message.source === 'model' && <div className="message-feedback">
              <button className={message.rating === 'up' ? 'selected' : ''} disabled={feedbackBusy !== null} title="喜欢这条回答，加入训练素材" aria-label="喜欢这条回答" aria-pressed={message.rating === 'up'} onClick={() => void rate(message, message.rating === 'up' ? null : 'up')}><Icon name="up" size={14} /></button>
              <button className={message.rating === 'down' ? 'selected' : ''} disabled={feedbackBusy !== null} title="这条回答不太好" aria-label="这条回答不太好" aria-pressed={message.rating === 'down'} onClick={() => void rate(message, message.rating === 'down' ? null : 'down')}><Icon name="down" size={14} /></button>
              <button className={message.correctedText ? 'selected feedback-edit' : 'feedback-edit'} onClick={() => { setEditing(message); setCorrection(message.correctedText ?? message.content) }}><Icon name="edit" size={14} />{message.correctedText ? '已写下理想回答' : '教她怎么回答'}</button>
            </div>}
          </div>
        </article>)}
        {busy && <article className="message message-assistant"><div className="portrait portrait-message"><img src="/avatar.png" alt="" /></div><div className="message-content"><div className="message-meta"><span>{settings.persona.name}</span><span>{cancelling ? '正在停止…' : stream ? '正在说…' : '正在想一想…'}</span></div><div className="message-bubble streaming" role="status">{stream || <span className="thinking-dots"><i /><i /><i /></span>}</div></div></article>}
      </div>}
      <div ref={end} />
    </section>

    <div className="composer-area">
      {error && <div className="notice notice-error" role="alert"><span>{error}</span><button onClick={() => setError('')} aria-label="关闭提示">×</button></div>}
      {IS_TAURI && status && !connected && !error && <div className="model-nudge">{status.error || `尚未找到 ${settings.model}。请在设置中检查本地模型。`}<button onClick={() => void openSettings()}>检查设置 <Icon name="arrow" size={14} /></button></div>}
      <form className="composer" onSubmit={e => { e.preventDefault(); void send() }}>
        <textarea ref={input} aria-label="想对她说的话" placeholder="想对她说点什么？" value={text} maxLength={12000} rows={2} onChange={e => setText(e.target.value)} onKeyDown={e => {
          if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing && e.nativeEvent.keyCode !== 229) { e.preventDefault(); void send() }
        }} />
        <div className="composer-toolbar"><span><Icon name="leaf" size={14} />{settings.model} <span className="composer-key-hint">· Enter 发送，Shift + Enter 换行</span></span>{busy ? <button type="button" className="send-button stop-button" onClick={() => void cancel()} disabled={cancelling} aria-label="停止生成"><Icon name="stop" size={17} />停止</button> : <button type="submit" className="send-button" disabled={!text.trim()} aria-label="发送消息"><Icon name="send" size={18} /></button>}</div>
      </form>
      <p className="composer-note">对话单独保存。说「记住：…」可以把重要的小事留在她的记忆里。</p>
    </div>

    {editing && <div className="modal-backdrop" role="presentation" onClick={e => e.target === e.currentTarget && !feedbackBusy && setEditing(null)}><section className="feedback-dialog" role="dialog" aria-modal="true" aria-labelledby="feedback-title">
      <div className="section-kicker"><Icon name="leaf" size={17} /> 一点点，更懂你</div><h2 id="feedback-title">你希望她怎样回答？</h2><p>写下理想的表达，作为之后 LoRA 训练的素材。保存不会立即改变模型。</p>
      <textarea autoFocus aria-label="理想回答" value={correction} maxLength={16000} onChange={e => setCorrection(e.target.value)} rows={7} />
      <div className="dialog-actions"><button className="secondary-button" onClick={() => setEditing(null)} disabled={!!feedbackBusy}>取消</button><button className="primary-button" disabled={!correction.trim() || !!feedbackBusy} onClick={() => void rate(editing, editing.rating, correction.trim())}>{feedbackBusy ? '保存中…' : '保存理想回答'}</button></div>
    </section></div>}
  </main>
}
