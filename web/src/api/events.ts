/**
 * The SSE client (spec/11 §4.2, spec/10 §3).
 *
 * One `EventSource` per open book (or global when no book is open). Deltas and previews are
 * coalesced with `requestAnimationFrame` before they reach the store.
 */
import type { QueryClient } from "@tanstack/react-query"

import { useLiveStore } from "@/stores/live"

type ServerEvent = {
  type: string
  data?: unknown
}

type Handler = (event: ServerEvent) => void

const handlers = new Set<Handler>()
let source: EventSource | null = null
let currentBookId: number | null = null
let hadError = false

/** Registers an event handler for the lifetime of the subscriber. */
export function subscribe(handler: Handler): () => void {
  handlers.add(handler)
  return () => {
    handlers.delete(handler)
  }
}

/**
 * Opens the event stream, re-creating it when the book changes.
 * Queries are invalidated after a reconnect because events are not replayed (spec/10 §3).
 */
export function connectEvents(bookId: number | null, queryClient: QueryClient) {
  if (source && currentBookId === bookId) {
    return
  }
  disconnectEvents()
  currentBookId = bookId

  const url = bookId === null ? "/api/events" : `/api/events?book_id=${bookId}`
  const store = useLiveStore.getState()
  store.setConnection("connecting")

  const eventSource = new EventSource(url)
  source = eventSource

  eventSource.onopen = () => {
    useLiveStore.getState().setConnection("open")
    if (hadError) {
      hadError = false
      void queryClient.invalidateQueries()
    }
  }

  eventSource.onerror = () => {
    hadError = true
    useLiveStore.getState().setConnection("reconnecting")
  }

  // `message` covers unnamed events; the server names every event, so listen on a generic
  // handler installed for each known type (spec/10 §3).
  const types = [
    "session.updated",
    "session.message",
    "session.delta",
    "session.tool_call",
    "session.tool_preview",
    "book.updated",
    "chapter.updated",
    "chapter.content",
    "canon.changed",
    "settings.changed",
    "proposal.changed",
    "knowledge.changed",
    "research.updated",
    "index.status",
    "resync",
  ]
  for (const type of types) {
    eventSource.addEventListener(type, (raw) => {
      const event = raw as MessageEvent<string>
      let data: unknown
      try {
        data = JSON.parse(event.data) as unknown
      } catch {
        data = null
      }
      dispatch({ type, data })
    })
  }
}

/** Closes the stream. */
export function disconnectEvents() {
  if (source) {
    source.close()
    source = null
  }
  currentBookId = null
}

function dispatch(event: ServerEvent) {
  for (const handler of handlers) {
    handler(event)
  }
}
