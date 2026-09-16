import { PetCanvas } from './body/PetCanvas'

/**
 * Pet 窗口的根组件。Phase 0：只有一个 Canvas 在呼吸。
 * Body 不含业务逻辑——它订阅 Core 的 `pet:state`（Phase 2 起），此处先用默认状态。
 */
export function App() {
  return <PetCanvas />
}
