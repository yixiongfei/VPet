import { useEffect, useState } from 'react'
import { IS_TAURI } from '../body/ipc'
import { DEFAULT_CHAT_SETTINGS, errorText, invokeStrict } from '../chat/api'
import type { ChatSettings, DesktopSettings, ModelStatus, Persona } from '../chat/api'
import { Icon } from '../chat/Icons'

interface Gift { id: string; name: string; feeling: number; strengthFood: number; strengthDrink: number; price?: number }
type Tab = 'desktop' | 'persona' | 'model' | 'gifts'
const tabs: Array<{ id: Tab; label: string; icon: 'settings' | 'spark' | 'leaf' | 'gift' }> = [
  { id: 'desktop', label: '桌面陪伴', icon: 'settings' }, { id: 'persona', label: '她的个性', icon: 'spark' },
  { id: 'model', label: '模型与学习', icon: 'leaf' }, { id: 'gifts', label: '送份心意', icon: 'gift' },
]
export function CompanionSettings() {
  const [tab, setTab] = useState<Tab>('desktop')
  const [desktop, setDesktop] = useState<DesktopSettings>({ size: 500, alwaysOnTop: true })
  const [settings, setSettings] = useState<ChatSettings>(DEFAULT_CHAT_SETTINGS)
  const [modelStatus, setModelStatus] = useState<ModelStatus | null>(null)
  const [gifts, setGifts] = useState<Gift[]>([])
  const [selectedGift, setSelectedGift] = useState('')
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState('')
  const [error, setError] = useState('')
  useEffect(() => {
    if (!IS_TAURI) return
    let live = true
    void Promise.allSettled([
      invokeStrict<DesktopSettings>('get_desktop_settings').then(s => live && setDesktop(s)),
      invokeStrict<ChatSettings>('get_chat_settings').then(s => live && setSettings(s)),
      invokeStrict<ModelStatus>('get_model_status').then(s => live && setModelStatus(s)),
      invokeStrict<Gift[]>('list_gifts').then(items => { if (live) { setGifts(items); setSelectedGift(items[0]?.id ?? '') } }),
    ]).then(results => {
      const failed = results.find(r => r.status === 'rejected')
      if (live && failed?.status === 'rejected') setError(errorText(failed.reason))
    })
    return () => { live = false }
  }, [])
  const action = async (operation: () => Promise<string>) => {
    if (busy) return
    setBusy(true); setNotice(''); setError('')
    try { setNotice(await operation()) } catch (e) { setError(errorText(e)) } finally { setBusy(false) }
  }
  const saveSettings = () => action(async () => {
    setSettings(await invokeStrict<ChatSettings>('save_chat_settings', { settings }))
    setModelStatus(await invokeStrict<ModelStatus>('get_model_status'))
    return '已保存，下次对话会使用新的设定。'
  })
  const personaField = (field: keyof Persona, value: string) => setSettings(s => ({ ...s, persona: { ...s.persona, [field]: value } }))
  const gift = gifts.find(g => g.id === selectedGift)
  return <section className="settings-surface">
    <nav className="settings-tabs" aria-label="设置类别">{tabs.map(item => <button key={item.id} className={tab === item.id ? 'active' : ''} aria-current={tab === item.id ? 'page' : undefined} onClick={() => { setTab(item.id); setNotice(''); setError('') }}><Icon name={item.icon} size={17} />{item.label}</button>)}</nav>
    <div className="settings-content">
      {tab === 'desktop' && <>
        <div className="section-kicker">YOUR LITTLE CORNER</div><h2>刚刚好的陪伴距离</h2><p className="section-description">调到喜欢的大小，让她待在你觉得舒服的位置。</p>
        <div className="desktop-preview"><div className="mini-desktop"><span /><span /><span /><div className="mini-document"><i /><i /><i /></div><img src="/avatar.png" alt="人物大小示意" style={{ width: 42 + desktop.size / 8 }} /></div><div><strong>{desktop.size} <small>px</small></strong><p>人物窗口大小</p></div></div>
        <label className="field-label" htmlFor="pet-size">显示大小<span>{Math.round(desktop.size / 5)}%</span></label>
        <input id="pet-size" className="range-input" type="range" min="200" max="800" step="10" value={desktop.size} onChange={e => setDesktop(s => ({ ...s, size: Number(e.target.value) }))} />
        <div className="range-captions"><span>小巧 · 200 px</span><span>放大 · 800 px</span></div>
        <div className="size-presets">{[[300, '小巧'], [500, '标准'], [650, '放大']].map(([size, label]) => <button key={size} className={desktop.size === size ? 'selected' : ''} onClick={() => setDesktop(s => ({ ...s, size: Number(size) }))}>{label}</button>)}</div>
        <label className="toggle-row"><span><strong>始终在最上层</strong><small>开启后，她会显示在其他窗口上方。</small></span><input type="checkbox" role="switch" checked={desktop.alwaysOnTop} onChange={e => setDesktop(s => ({ ...s, alwaysOnTop: e.target.checked }))} /><span className="switch-track" /></label>
        <div className="setting-tips"><Icon name="chat" size={17} /><p>单击人物打开对话，按住并移动即可拖拽。<br />在人物上右键打开这个设置页；托盘图标右键有快捷菜单。</p></div>
        <button className="primary-button" disabled={busy} onClick={() => void action(async () => { setDesktop(await invokeStrict<DesktopSettings>('set_desktop_settings', { settings: desktop })); return '显示设置已应用，下次启动也会保留。' })}>{busy ? '应用中…' : '应用显示设置'}<Icon name="check" size={16} /></button>
      </>}

      {tab === 'persona' && <>
        <div className="section-kicker">A PERSONALITY OF HER OWN</div><h2>一点点，成为你熟悉的她</h2><p className="section-description">把背景、性格和表达习惯告诉她。这些设定会参与之后的每次对话。</p>
        <div className="persona-heading"><div className="portrait portrait-settings"><img src="/avatar.png" alt="角色头像" /></div><label className="form-field">她的名字<input value={settings.persona.name} maxLength={40} onChange={e => personaField('name', e.target.value)} placeholder="你想怎么称呼她" /></label></div>
        <label className="form-field">背景故事<textarea value={settings.persona.background} maxLength={3000} rows={3} onChange={e => personaField('background', e.target.value)} placeholder="她来自哪里，你们如何相识…" /></label>
        <label className="form-field">形象设定<textarea value={settings.persona.appearance} maxLength={2000} rows={2} onChange={e => personaField('appearance', e.target.value)} /><small>用于对话中的角色描述；当前人物动画外观保持原样。</small></label>
        <div className="field-columns"><label className="form-field">性格<textarea value={settings.persona.personality} maxLength={2000} rows={4} onChange={e => personaField('personality', e.target.value)} /></label><label className="form-field">说话方式<textarea value={settings.persona.speakingStyle} maxLength={2000} rows={4} onChange={e => personaField('speakingStyle', e.target.value)} /></label></div>
        <button className="primary-button" disabled={busy || !settings.persona.name.trim()} onClick={() => void saveSettings()}>{busy ? '保存中…' : '保存她的设定'}<Icon name="check" size={16} /></button>
      </>}

      {tab === 'model' && <>
        <div className="section-kicker">LOCAL & PERSONAL</div><h2>慢慢更懂你</h2><p className="section-description">连接这台电脑上的模型，让真实的对话从这里开始。</p>
        <div className={`connection-card ${modelStatus?.connected ? 'connected' : ''}`}><span className={`status-dot ${modelStatus?.connected ? 'is-online' : ''}`} /><div><strong>{!IS_TAURI ? '浏览器预览' : modelStatus?.connected ? 'Ollama 已连接' : '本地模型未连接'}</strong><p>{!IS_TAURI ? '在桌宠应用中检查真实的连接状态。' : modelStatus?.connected ? `找到 ${modelStatus.models.length} 个本地模型` : modelStatus?.error || '点击下方按钮检查连接。'}</p></div><button className="secondary-button" disabled={busy} onClick={() => void action(async () => { const result = await invokeStrict<ModelStatus>('get_model_status'); setModelStatus(result); if (!result.connected) throw new Error(result.error || '无法连接 Ollama，请确认本地服务已启动。'); return '已检查保存的连接配置。' })}>检查连接</button></div>
        <label className="form-field">模型名称<input list="local-models" value={settings.model} maxLength={200} placeholder="qwen3.5:9b" onChange={e => setSettings(s => ({ ...s, model: e.target.value }))} /><datalist id="local-models">{modelStatus?.models.map(model => <option key={model} value={model} />)}</datalist><small>可选择已下载的模型，或填入你训练、导入后的模型名称。</small></label>
        <label className="form-field">Ollama 本地地址<input value={settings.endpoint} maxLength={300} placeholder="http://127.0.0.1:11434" onChange={e => setSettings(s => ({ ...s, endpoint: e.target.value }))} /><small>仅使用本机地址。更改后先保存，再检查连接。</small></label>
        <label className="field-label" htmlFor="model-temperature">表达自由度<span>{settings.temperature.toFixed(2)}</span></label><input id="model-temperature" className="range-input" type="range" min="0" max="1.5" step="0.05" value={settings.temperature} onChange={e => setSettings(s => ({ ...s, temperature: Number(e.target.value) }))} /><div className="range-captions"><span>更稳定</span><span>更活泼</span></div>
        <button className="primary-button" disabled={busy || !settings.model.trim()} onClick={() => void saveSettings()}>{busy ? '保存中…' : '保存模型设置'}<Icon name="check" size={16} /></button>
        <div className="training-card"><div className="section-kicker"><Icon name="leaf" size={16} /> 回答反馈 · LoRA 素材</div><h3>把喜欢的表达，慢慢留下来</h3><p>在对话中点赞，或点击「教她怎么回答」写下理想回答。导出的训练集只包含这些明确选择的样本。</p><p className="muted">反馈用于积累训练素材，不会立刻改变模型。导出后可使用项目中的 LoRA 脚本训练，再在上方切换模型。</p><button className="secondary-button" disabled={busy} onClick={() => void action(async () => { const exported = await invokeStrict<{ path: string; samples: number }>('export_training_data'); return `已导出 ${exported.samples} 条训练样本：${exported.path}` })}>导出训练样本<Icon name="arrow" size={15} /></button></div>
      </>}

      {tab === 'gifts' && <>
        <div className="section-kicker">A LITTLE SOMETHING FOR HER</div><h2>送她一份小心意</h2><p className="section-description">选一份礼物，把平常的一天变得特别一点。</p>
        <div className="gift-illustration"><Icon name="gift" size={56} /><span>FOR YOU</span></div>
        {gifts.length ? <><label className="form-field">挑选礼物<select value={selectedGift} onChange={e => setSelectedGift(e.target.value)}>{gifts.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</select></label>{gift && <div className="gift-effects"><span>心情 +{Math.max(0, gift.feeling).toFixed(0)}</span>{gift.strengthFood > 0 && <span>饱腹 +{gift.strengthFood.toFixed(0)}</span>}{gift.strengthDrink > 0 && <span>水分 +{gift.strengthDrink.toFixed(0)}</span>}{gift.price != null && <span>价格 {gift.price}</span>}</div>}<button className="primary-button" disabled={busy || !selectedGift} onClick={() => void action(async () => { const name = await invokeStrict<string>('give_gift', { id: selectedGift }); return `送出了「${name}」，看看她的反应吧。` })}>{busy ? '正在送出…' : '送给她'}<Icon name="gift" size={17} /></button></> : <div className="empty-state"><p>{!IS_TAURI ? '打开桌宠应用后，这里会显示可赠送的礼物。' : '礼物列表还没准备好，等人物资源加载后再试一次。'}</p><button className="secondary-button" disabled={busy} onClick={() => void action(async () => { const items = await invokeStrict<Gift[]>('list_gifts'); setGifts(items); setSelectedGift(items[0]?.id ?? ''); return items.length ? '礼物列表已刷新。' : '还没有可赠送的礼物，请检查人物资源。' })}>刷新礼物</button></div>}
      </>}
      {notice && <div className="notice notice-success" role="status"><Icon name="check" size={16} /><span>{notice}</span></div>}
      {error && <div className="notice notice-error" role="alert"><span>{error}</span></div>}
    </div>
  </section>
}
