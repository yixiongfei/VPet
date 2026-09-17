import React from 'react'
import ReactDOM from 'react-dom/client'
import { IS_TAURI, invokeCore } from '../body/ipc'
import { Icon } from '../chat/Icons'
import '../chat/companion.css'
import { CompanionSettings } from './CompanionSettings'
import { Panel } from './Panel'

/**
 * 设置页：上面是给用户看的「陪伴设置」（显示大小 / 个性 / 模型 / 礼物），
 * 下面折叠着原来的调试面板——数值、记忆、工具调用都还在，只是不再是首屏。
 */
function SettingsPage() {
  const openChat = () => {
    if (!IS_TAURI) { window.location.href = '/chat.html'; return }
    void invokeCore('open_chat')
  }
  return (
    <div className="settings-page">
      <header className="settings-page-header">
        <div>
          <h1>陪伴设置</h1>
          <p>调整她在桌面上的样子、个性和本地模型。所有设置都只保存在这台电脑。</p>
        </div>
        <button className="secondary-button" onClick={openChat}>
          <Icon name="chat" size={16} />和她聊聊
        </button>
      </header>
      <CompanionSettings />
      <details className="legacy-details">
        <summary>状态与调试面板（数值 · 记忆 · 计时器 · 工具调用）</summary>
        <div className="legacy-panel">
          <Panel />
        </div>
      </details>
      <footer className="settings-footer">VPet · 本地陪伴，不上传任何数据</footer>
    </div>
  )
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <SettingsPage />
  </React.StrictMode>,
)
