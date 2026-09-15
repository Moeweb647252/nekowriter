import { createContext, useContext } from "react"

import { type GenerationPreviewBuffer } from "@/components/GenerationPreview"

/** 事件同步连接状态。 */
export type EventConnection = "connecting" | "connected" | "reconnecting"

export type EventSyncState = {
  connection: EventConnection
  /** 已确认本机服务不再应答：界面据此显示「已退出」并停止重连。 */
  serviceStopped: boolean
  /** 退出动作或健康检查确认服务已退出时调用，只记录一次。 */
  markServiceStopped: () => void
  knowledgeVersion: number
  previews: Record<string, GenerationPreviewBuffer>
  recycleVersion: number
  settingsVersion: number
  templateVersion: number
}

/**
 * 事件同步的上下文与读取它的 Hook。
 *
 * 它们单独成文件而不留在 `events.tsx`：与组件放在同一个模块会让 React Fast Refresh
 * 在改动这个文件时整页刷新，也会让「一个文件只导出组件」的约定失真。
 */
export const EventSyncContext = createContext<EventSyncState | null>(null)

export function useEventSync() {
  const context = useContext(EventSyncContext)
  if (!context) {
    throw new Error("useEventSync must be used within EventSyncProvider")
  }
  return context
}
