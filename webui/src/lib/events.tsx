import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react"
import { useQueryClient } from "@tanstack/react-query"

import { type GenerationPreviewBuffer } from "@/components/GenerationPreview"
import {
  EventSyncContext,
  type EventConnection,
} from "@/lib/event-sync-context"
import {
  getEventCursor,
  getHealth,
  type PersistedEvent,
  type PreviewDelta,
} from "@/lib/api"

/**
 * 后端会写出的持久事件类型，必须与 `crates/nekowriter-store/src/event_types.rs` 的
 * 清单逐项相同（少一个就会静默不刷新）；这条等式由 store 的
 * `frontend_subscribes_to_exactly_the_manifest` 钉住。
 */
const persistentEventTypes = [
  "chapter.changed",
  "document.committed",
  "feedback.changed",
  "knowledge.index_changed",
  "model_request.cancelled",
  "project.changed",
  "prompt_template.changed",
  "question.answered",
  "question.created",
  "recycle.changed",
  "review.changed",
  "run.changed",
  "settings.changed",
  "system.model_operation",
  "tool.included",
  "tool.proposed",
  "tool.resolved",
  "version.completed",
  "workflow.changed",
] as const

function previewBufferKey(delta: PreviewDelta) {
  return delta.tool
    ? `${delta.requestId}:tool:${delta.tool.index}`
    : `${delta.requestId}:text`
}

function previewOffset(value: string) {
  try {
    const offset = BigInt(value)
    if (offset < 0n || offset > BigInt(Number.MAX_SAFE_INTEGER)) return null
    return Number(offset)
  } catch {
    return null
  }
}

/** 只接受连续追加或已收到的重复片段，避免乱序临时流伪装成正文。 */
function appendPreviewContent(
  existing: string,
  offset: number,
  incoming: string
) {
  const characters = Array.from(existing)
  if (offset > characters.length) return null
  if (offset < characters.length) {
    const seen = characters
      .slice(offset, offset + Array.from(incoming).length)
      .join("")
    return seen === incoming ? existing : null
  }
  return existing + incoming
}

function mergePreviewDelta(
  current: Record<string, GenerationPreviewBuffer>,
  delta: PreviewDelta
) {
  const offset = previewOffset(delta.offset)
  if (offset === null) return current
  const key = previewBufferKey(delta)
  const previous = current[key]
  const content = appendPreviewContent(
    previous?.content ?? "",
    offset,
    delta.content
  )
  if (content === null) return current
  return {
    ...current,
    [key]: {
      key,
      versionId: delta.versionId,
      requestId: delta.requestId,
      tool: delta.tool
        ? {
            index: delta.tool.index,
            id: delta.tool.id ?? previous?.tool?.id,
            name: delta.tool.name ?? previous?.tool?.name,
          }
        : undefined,
      content,
    },
  }
}

function discardPreviewForRequest(
  current: Record<string, GenerationPreviewBuffer>,
  requestId: string
) {
  let changed = false
  const next: Record<string, GenerationPreviewBuffer> = {}
  for (const [key, preview] of Object.entries(current)) {
    if (preview.requestId === requestId) {
      changed = true
    } else {
      next[key] = preview
    }
  }
  return changed ? next : current
}

function requestIdFrom(event: PersistedEvent) {
  if (!event.payload || typeof event.payload !== "object") return null
  const value = (event.payload as { request_id?: unknown }).request_id
  return typeof value === "string" ? value : null
}

/**
 * 一个标签页只保持一条可恢复事件流。持久事件只失效查询缓存，临时预览仅留在内存；
 * 因此重新连接或切换作品都不会把半截模型输出当成已保存内容。
 */
export function EventSyncProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient()
  const [connection, setConnection] = useState<EventConnection>("connecting")
  const [serviceStopped, setServiceStopped] = useState(false)
  const serviceStoppedRef = useRef(false)
  const markServiceStopped = useCallback(() => {
    if (serviceStoppedRef.current) return
    serviceStoppedRef.current = true
    setServiceStopped(true)
  }, [])
  const [knowledgeVersion, setKnowledgeVersion] = useState(0)
  const [previews, setPreviews] = useState<
    Record<string, GenerationPreviewBuffer>
  >({})
  const [recycleVersion, setRecycleVersion] = useState(0)
  const [settingsVersion, setSettingsVersion] = useState(0)
  const [templateVersion, setTemplateVersion] = useState(0)
  const seenSequences = useRef(new Set<string>())

  useEffect(() => {
    let disposed = false
    let source: EventSource | null = null
    let resetting = false

    /**
     * 事件流断开后确认服务是否还在。
     *
     * 连不上不等于服务退出：也可能是同一条流正在重连。因此先问一次健康检查，
     * 只有确认不可达才切到「已退出」并停止重连。
     */
    const confirmServiceAlive = async () => {
      if (serviceStoppedRef.current) return false
      const alive = await getHealth().then(
        () => true,
        () => false
      )
      if (!alive) markServiceStopped()
      return alive
    }

    const invalidateFromEvent = (event: PersistedEvent, eventType: string) => {
      queryClient.invalidateQueries({ queryKey: ["projects"] })
      queryClient.invalidateQueries({ queryKey: ["dashboard"] })
      if (event.version_id) {
        queryClient.invalidateQueries({
          queryKey: ["version", event.version_id],
        })
        queryClient.invalidateQueries({
          queryKey: ["preparation", event.version_id],
        })
        queryClient.invalidateQueries({
          queryKey: ["chapters", event.version_id],
        })
      }
      if (eventType === "settings.changed") {
        queryClient.invalidateQueries({ queryKey: ["settings"] })
      }
      if (eventType === "knowledge.index_changed") {
        queryClient.invalidateQueries({ queryKey: ["knowledge"] })
      }
      if (eventType === "prompt_template.changed") {
        queryClient.invalidateQueries({ queryKey: ["templates"] })
      }
      if (eventType === "recycle.changed") {
        queryClient.invalidateQueries({ queryKey: ["recycle"] })
      }
    }

    const connect = async () => {
      if (serviceStoppedRef.current) return
      setConnection((current) =>
        current === "connected" ? "reconnecting" : "connecting"
      )
      try {
        const cursor = await getEventCursor()
        if (disposed) return
        const nextSource = new EventSource(
          `/api/events?after=${encodeURIComponent(cursor.eventWatermark)}`
        )
        source = nextSource
        nextSource.onopen = () => {
          if (!disposed && source === nextSource) setConnection("connected")
        }
        nextSource.onerror = () => {
          if (disposed || source !== nextSource) return
          setPreviews({})
          void confirmServiceAlive().then((alive) => {
            if (disposed || source !== nextSource) return
            if (alive) {
              setConnection("reconnecting")
              return
            }
            // 服务已经退出：关掉事件源，浏览器就不会再自动重连。
            nextSource.close()
          })
        }
        const reset = () => {
          if (disposed || resetting || source !== nextSource) return
          resetting = true
          nextSource.close()
          seenSequences.current.clear()
          setPreviews({})
          queryClient.invalidateQueries()
          setKnowledgeVersion((current) => current + 1)
          setRecycleVersion((current) => current + 1)
          setSettingsVersion((current) => current + 1)
          setTemplateVersion((current) => current + 1)
          void connect().finally(() => {
            resetting = false
          })
        }
        const onPersistent = (message: Event, eventType: string) => {
          try {
            const event = JSON.parse(
              (message as MessageEvent<string>).data
            ) as PersistedEvent
            if (
              disposed ||
              source !== nextSource ||
              seenSequences.current.has(event.seq)
            ) {
              return
            }
            seenSequences.current.add(event.seq)
            if (seenSequences.current.size > 2_048) {
              const oldest = seenSequences.current.values().next().value
              if (oldest) seenSequences.current.delete(oldest)
            }
            const requestId = requestIdFrom(event)
            if (requestId) {
              setPreviews((current) =>
                discardPreviewForRequest(current, requestId)
              )
            }
            invalidateFromEvent(event, eventType)
            if (eventType === "settings.changed") {
              setSettingsVersion((current) => current + 1)
            }
            if (eventType === "knowledge.index_changed") {
              setKnowledgeVersion((current) => current + 1)
            }
            if (eventType === "prompt_template.changed") {
              setTemplateVersion((current) => current + 1)
            }
            if (eventType === "recycle.changed") {
              setRecycleVersion((current) => current + 1)
            }
          } catch {
            // 损坏的单条事件不能中断后续有序同步。
          }
        }
        for (const eventType of persistentEventTypes) {
          nextSource.addEventListener(eventType, (event) =>
            onPersistent(event, eventType)
          )
        }
        nextSource.addEventListener("preview.delta", (message) => {
          try {
            const delta = JSON.parse(
              (message as MessageEvent<string>).data
            ) as PreviewDelta
            if (
              disposed ||
              source !== nextSource ||
              typeof delta.requestId !== "string" ||
              typeof delta.offset !== "string" ||
              typeof delta.content !== "string"
            ) {
              return
            }
            setPreviews((current) => mergePreviewDelta(current, delta))
          } catch {
            // 临时预览损坏时直接丢弃，不影响持久状态。
          }
        })
        nextSource.addEventListener("stream.reset_required", reset)
      } catch {
        if (disposed) return
        if (await confirmServiceAlive()) setConnection("reconnecting")
      }
    }

    void connect()
    return () => {
      disposed = true
      source?.close()
    }
  }, [markServiceStopped, queryClient])

  const value = useMemo(
    () => ({
      connection,
      knowledgeVersion,
      markServiceStopped,
      previews,
      recycleVersion,
      serviceStopped,
      settingsVersion,
      templateVersion,
    }),
    [
      connection,
      knowledgeVersion,
      markServiceStopped,
      previews,
      recycleVersion,
      serviceStopped,
      settingsVersion,
      templateVersion,
    ]
  )
  return (
    <EventSyncContext.Provider value={value}>
      {children}
    </EventSyncContext.Provider>
  )
}
