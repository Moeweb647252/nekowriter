import { useQueryClient } from "@tanstack/react-query"
import { type ReactNode, useEffect, useState } from "react"

import { systemApi } from "../endpoints/system"
import type { ServerMessage } from "../generated/ServerMessage"
import { type ConnectionStatus, ConnectionContext } from "./connection"
import { invalidateFor } from "./invalidation"
import { PreviewContext, PreviewStore } from "./preview"
import { assertNever } from "@/lib/assert"

/** How long to wait before trying again when the service cannot be reached. */
const RETRY_DELAY_MS = 2000

/**
 * Keeps cached server data current by following the event stream.
 *
 * The cursor is read before any data is loaded (children render only afterwards), and
 * events after it are followed, so no change between loading and subscribing is missed.
 * When the stream asks for a reset, the cursor is read again and all data is reloaded.
 */
export function EventStreamProvider({
  children,
  splash,
}: {
  children: ReactNode
  /** Shown until the first cursor has been read. */
  splash: ReactNode
}) {
  const queryClient = useQueryClient()
  const [status, setStatus] = useState<ConnectionStatus>("connecting")
  const [previews] = useState(() => new PreviewStore())

  useEffect(() => {
    let source: EventSource | null = null
    let retryTimer: ReturnType<typeof setTimeout> | undefined
    let disposed = false

    const connect = async (reloadData: boolean) => {
      let cursor: number
      try {
        cursor = (await systemApi.eventCursor()).cursor
      } catch {
        if (disposed) return
        setStatus((current) => (current === "connecting" ? current : "reconnecting"))
        retryTimer = setTimeout(() => void connect(reloadData), RETRY_DELAY_MS)
        return
      }
      if (disposed) return
      if (reloadData) await queryClient.invalidateQueries()
      source = new EventSource(`/api/events?after=${cursor}`)
      setStatus("live")
      source.onopen = () => setStatus("live")
      source.onmessage = (message: MessageEvent<string>) => {
        handle(JSON.parse(message.data) as ServerMessage)
      }
      source.onerror = () => {
        // A preview may have ended while disconnected; the stream resends a current one.
        previews.set(null)
        if (source?.readyState === EventSource.CLOSED) {
          // The browser gave up reconnecting; start over from a fresh cursor.
          source = null
          setStatus("reconnecting")
          retryTimer = setTimeout(() => void connect(true), RETRY_DELAY_MS)
        } else {
          // The browser reconnects by itself and resumes with Last-Event-ID.
          setStatus("reconnecting")
        }
      }
    }

    const handle = (message: ServerMessage) => {
      switch (message.type) {
        case "event":
          void invalidateFor(queryClient, message.event)
          return
        case "preview":
          previews.set(message.preview)
          return
        case "reset_required":
          source?.close()
          source = null
          void connect(true)
          return
        case "shutting_down":
          source?.close()
          source = null
          previews.set(null)
          setStatus("stopped")
          return
        default:
          assertNever(message)
      }
    }

    void connect(false)
    return () => {
      disposed = true
      clearTimeout(retryTimer)
      source?.close()
    }
  }, [queryClient, previews])

  return (
    <ConnectionContext value={status}>
      <PreviewContext value={previews}>
        {status === "connecting" ? splash : children}
      </PreviewContext>
    </ConnectionContext>
  )
}
