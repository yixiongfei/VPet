import type { CSSProperties } from 'react'

type Name = 'send' | 'settings' | 'chat' | 'spark' | 'leaf' | 'gift' | 'up' | 'down' | 'edit' | 'stop' | 'arrow' | 'check'
const shapes: Record<Name, React.ReactNode> = {
  send: <><path d="m4 12 16-8-5 16-3-6-8-2Z"/><path d="m12 14 8-10"/></>,
  settings: <><path d="M4 7h16M4 17h16"/><circle cx="9" cy="7" r="3"/><circle cx="16" cy="17" r="3"/></>,
  chat: <path d="M20 11a8 8 0 0 1-8 8H5l-3 3V11a9 9 0 0 1 18 0Z"/>,
  spark: <><path d="m12 3 2.2 6.8L21 12l-6.8 2.2L12 21l-2.2-6.8L3 12l6.8-2.2L12 3Z"/></>,
  leaf: <><path d="M20 3C8 3 3 8 5 15s16 7 15-12Z"/><path d="M3 21 15 9"/></>,
  gift: <><path d="M4 11h16v10H4zM2 7h20v4H2zM12 7v14"/><path d="M12 7C3 8 5 0 9 3l3 4Zm0 0c9 1 7-7 3-4l-3 4Z"/></>,
  up: <path d="M7 10v11H3V10h4Zm0 1 5-8c2 0 3 1 2 6h5c2 0 2 2 1 5l-2 7H7"/>,
  down: <path d="M7 14V3H3v11h4Zm0-1 5 8c2 0 3-1 2-6h5c2 0 2-2 1-5l-2-7H7"/>,
  edit: <><path d="m15 4 5 5M4 20l5-1L21 7l-4-4L5 15l-1 5Z"/></>,
  stop: <rect x="6" y="6" width="12" height="12" rx="2"/>,
  arrow: <><path d="M5 12h14m-6-6 6 6-6 6"/></>,
  check: <path d="m5 12 4 4L19 6"/>,
}
export function Icon({ name, size = 18, style }: { name: Name; size?: number; style?: CSSProperties }) {
  return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.65" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" style={style}>{shapes[name]}</svg>
}
