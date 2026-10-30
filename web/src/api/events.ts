/**
 * The SSE client (spec/11 §4.2, spec/10 §3).
 *
 * One `EventSource` per open book (or global when no book is open). Deltas and previews are
 * coalesced with `requestAnimationFrame` before they reach the store.
 */
import type { QueryClient } from "@tanstack/react-query"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { ContentOp } from "@/api/gen/ContentOp"
import { keys } from "@/api/keys"
import { useChapterDoc } from "@/stores/chapterDoc"
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
      applyServerEvent(queryClient, type, data)
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
  flushLiveDeltas()
}

function dispatch(event: ServerEvent) {
  for (const handler of handlers) {
    handler(event)
  }
}

/** The parsed payload of an SSE frame. */
export type EventPayload = unknown

/** Narrow a parsed payload to a record, or `null`. */
function payloadRecord(payload: unknown): Record<string, unknown> | null {
  return typeof payload === "object" && payload !== null
    ? (payload as Record<string, unknown>)
    : null
}

// ───────────────────────── delta coalescing (spec/11 §4.2) ─────────────────────────
//
// A streaming turn produces dozens of `session.delta`/`session.tool_preview` frames per second.
// Applying each one as its own store write re-renders the chapter (ghost text) and the activity
// stream once per byte; batching them into one animation frame keeps a 300-paragraph chapter
// smooth while it streams (PLAN 11.3). The buffers are flushed on the next frame, and a test or a
// teardown can flush them synchronously with `flushLiveDeltas`.

type PendingDelta = {
  sessionId: number
  seq: number
  kind: "text" | "reasoning"
  index: number
  text: string
}

type PendingPreview = {
  sessionId: number
  seq: number
  callIndex: number
  name: string
  field: string
  text: string
}

let pendingDeltas: PendingDelta[] = []
let pendingPreviews: PendingPreview[] = []
let frameHandle: number | null = null
let timerHandle: ReturnType<typeof setTimeout> | null = null

/** Applies everything buffered in the current frame immediately. */
export function flushLiveDeltas(): void {
  if (frameHandle !== null && typeof cancelAnimationFrame === "function") {
    cancelAnimationFrame(frameHandle)
  }
  frameHandle = null
  if (timerHandle !== null) {
    clearTimeout(timerHandle)
  }
  timerHandle = null

  const deltas = pendingDeltas
  pendingDeltas = []
  if (deltas.length > 0) {
    const merged = new Map<string, PendingDelta>()
    for (const delta of deltas) {
      const key = `${delta.sessionId}:${delta.seq}:${delta.kind}:${delta.index}`
      const current = merged.get(key)
      if (current === undefined) {
        merged.set(key, { ...delta })
      } else {
        current.text += delta.text
      }
    }
    const store = useLiveStore.getState()
    for (const delta of merged.values()) {
      store.applyDelta(delta.sessionId, delta.seq, delta.kind, delta.index, delta.text)
    }
  }

  const previews = pendingPreviews
  pendingPreviews = []
  if (previews.length > 0) {
    const merged = new Map<string, PendingPreview>()
    for (const preview of previews) {
      const key = `${preview.sessionId}:${preview.seq}:${preview.callIndex}:${preview.field}`
      const current = merged.get(key)
      if (current === undefined) {
        merged.set(key, { ...preview })
      } else {
        current.text += preview.text
      }
    }
    const store = useLiveStore.getState()
    for (const preview of merged.values()) {
      store.applyPreview(
        preview.sessionId,
        preview.seq,
        preview.callIndex,
        preview.name,
        preview.field,
        preview.text,
      )
    }
  }
}

/** Drops the buffers of one sequence (its persisted message supersedes them). */
function dropPending(sessionId: number, seq: number): void {
  pendingDeltas = pendingDeltas.filter(
    (delta) => !(delta.sessionId === sessionId && delta.seq === seq),
  )
  pendingPreviews = pendingPreviews.filter(
    (preview) => !(preview.sessionId === sessionId && preview.seq === seq),
  )
}

function scheduleFlush(): void {
  if (frameHandle !== null || timerHandle !== null) {
    return
  }
  if (typeof requestAnimationFrame === "function") {
    frameHandle = requestAnimationFrame(() => {
      frameHandle = null
      flushLiveDeltas()
    })
  } else {
    // Node/jsdom without the frame API: one tick is the closest equivalent.
    timerHandle = setTimeout(() => {
      timerHandle = null
      flushLiveDeltas()
    }, 16)
  }
}

/** The dispatch table (spec/11 §4.2): update stores and caches where the payload is complete,
 * invalidate where the event is only a signal.
 */
export function applyServerEvent(
  queryClient: QueryClient,
  type: string,
  payload: unknown
): void {
  const data = payloadRecord(payload)
  switch (type) {
    case "chapter.content": {
      if (data === null) {
        return
      }
      const chapterId = data.chapter_id
      const version = data.version
      const ops = data.ops
      const actor = data.actor
      if (
        typeof chapterId !== "number" ||
        typeof version !== "number" ||
        !Array.isArray(ops)
      ) {
        return
      }
      const state = useChapterDoc.getState()
      if (state.chapterId !== chapterId) {
        // The document belongs to another chapter: only the caches need refreshing.
        void queryClient.invalidateQueries({
          queryKey: keys.chapters.detail(chapterId),
        })
        return
      }
      const result = state.applyOps(
        version,
        ops as ContentOp[],
        actor === "agent" ? "agent" : "user"
      )
      if (result === "gap") {
        // Missed updates: the only correct recovery is a refetch.
        void queryClient.invalidateQueries({
          queryKey: keys.chapters.detail(chapterId),
        })
        return
      }
      // Keep the cached chapter in step with the store. The chapter page reloads the store when
      // the cached document changes, so a cache left one version behind would silently undo the
      // ops that were just applied.
      const after = useChapterDoc.getState()
      const cached = queryClient.getQueryData<ChapterDto>(
        keys.chapters.detail(chapterId)
      )
      if (cached !== undefined) {
        queryClient.setQueryData(keys.chapters.detail(chapterId), {
          ...cached,
          content: {
            ...cached.content,
            paragraphs: after.paragraphs,
            next_pid: after.nextPid,
            tail_open: after.tailOpen,
          },
          content_version: version,
          word_count:
            typeof data.word_count === "number"
              ? data.word_count
              : cached.word_count,
        })
      }
      return
    }
    case "chapter.updated": {
      if (data === null) {
        return
      }
      // A chapter becoming official can satisfy `chapter_at_least`; the evaluator is only as
      // fresh as its next read, so every goal list and the overview are invalidated.
      void queryClient.invalidateQueries({ queryKey: keys.canon.goalsAll })
      const chapter = payloadRecord(data.chapter)
      if (chapter === null) {
        return
      }
      const eventBookId = chapter.book_id
      if (typeof eventBookId === "number") {
        void queryClient.invalidateQueries({
          queryKey: keys.books.overview(eventBookId),
        })
      }
      const trackId = chapter.track_id
      if (typeof trackId === "number") {
        void queryClient.invalidateQueries({
          queryKey: keys.tracks.chapters(trackId),
        })
      }
      const chapterId = chapter.id
      if (typeof chapterId === "number") {
        if (data.outline_changed === true) {
          void queryClient.invalidateQueries({
            queryKey: keys.chapters.detail(chapterId),
          })
        }
        const cached = queryClient.getQueryData<ChapterDto>(
          keys.chapters.detail(chapterId)
        )
        if (cached !== undefined) {
          queryClient.setQueryData(keys.chapters.detail(chapterId), {
            ...cached,
            ...chapter,
          })
        }
      }
      return
    }
    case "book.updated": {
      if (data === null) {
        return
      }
      void queryClient.invalidateQueries({ queryKey: keys.books.all })
      const bookId = data.book_id
      if (typeof bookId === "number") {
        void queryClient.invalidateQueries({
          queryKey: keys.books.detail(bookId),
        })
        void queryClient.invalidateQueries({
          queryKey: keys.books.tracks(bookId),
        })
        void queryClient.invalidateQueries({
          queryKey: keys.books.overview(bookId),
        })
      }
      return
    }
    case "canon.changed": {
      if (data === null) {
        return
      }
      // A canon write changes the canon pages, the memory snapshot and the change log
      // (spec/10 §3). The event is only a signal, so the caches are invalidated, not patched.
      void queryClient.invalidateQueries({ queryKey: keys.canon.all })
      void queryClient.invalidateQueries({ queryKey: keys.changes.all })
      const bookId = data.book_id
      if (typeof bookId === "number") {
        void queryClient.invalidateQueries({
          queryKey: keys.books.overview(bookId),
        })
      }
      return
    }
    case "proposal.changed": {
      if (data === null) {
        return
      }
      // A maintenance run creates proposals and the author decides them: the proposals page and
      // the navigator badge must follow (spec/10 §3).
      void queryClient.invalidateQueries({ queryKey: keys.proposals.all })
      const bookId = data.book_id
      if (typeof bookId === "number") {
        void queryClient.invalidateQueries({
          queryKey: keys.books.overview(bookId),
        })
      }
      return
    }
    case "settings.changed": {
      if (data === null) {
        return
      }
      const bookId = data.book_id
      if (typeof bookId !== "number") {
        return
      }
      const kind = data.kind
      if (kind === "setting_doc") {
        void queryClient.invalidateQueries({
          queryKey: keys.settings.docs(bookId),
        })
        void queryClient.invalidateQueries({ queryKey: keys.changes.all })
      } else if (kind === "characters") {
        void queryClient.invalidateQueries({
          queryKey: keys.settings.characters(bookId),
        })
      }
      void queryClient.invalidateQueries({
        queryKey: keys.books.overview(bookId),
      })
      return
    }
    case "session.delta": {
      if (data === null) {
        return
      }
      const sessionId = data.session_id
      const seq = data.seq
      const kind = data.kind
      const index = data.index
      const text = data.text
      if (
        typeof sessionId !== "number" ||
        typeof seq !== "number" ||
        typeof index !== "number" ||
        typeof text !== "string" ||
        (kind !== "text" && kind !== "reasoning")
      ) {
        return
      }
      pendingDeltas.push({ sessionId, seq, kind, index, text })
      scheduleFlush()
      return
    }
    case "session.tool_preview": {
      if (data === null) {
        return
      }
      const sessionId = data.session_id
      const seq = data.seq
      const callIndex = data.call_index
      const name = data.name
      const field = data.field
      const delta = data.delta
      if (
        typeof sessionId !== "number" ||
        typeof seq !== "number" ||
        typeof callIndex !== "number" ||
        typeof name !== "string" ||
        typeof field !== "string" ||
        typeof delta !== "string"
      ) {
        return
      }
      pendingPreviews.push({ sessionId, seq, callIndex, name, field, text: delta })
      scheduleFlush()
      return
    }
    case "session.message": {
      if (data === null) {
        return
      }
      const sessionId = data.session_id
      const message = payloadRecord(data.message)
      if (typeof sessionId !== "number" || message === null) {
        return
      }
      // The persisted message supersedes the live buffers of its sequence (spec/11 §4.3).
      const seq = message.seq
      if (typeof seq === "number") {
        dropPending(sessionId, seq)
        useLiveStore.getState().clearSeq(sessionId, seq)
      }
      void queryClient.invalidateQueries({
        queryKey: keys.sessions.detail(sessionId),
      })
      return
    }
    case "session.updated":
    case "session.tool_call": {
      if (data === null) {
        return
      }
      const session = payloadRecord(data.session)
      const sessionId = data.session_id ?? session?.id
      if (typeof sessionId === "number") {
        void queryClient.invalidateQueries({
          queryKey: keys.sessions.detail(sessionId),
        })
      }
      // `ChapterDto.open_sessions` is this client's only handle on a chapter's writer session, and
      // it is cached under the chapter's own key (spec/10 §2.8). A session that the *agent* drives
      // into `held` — `auto_pause`, or `pause_after_outline` — produces no client mutation, so
      // without this the chapter kept `status: "running"`, `useWriterState` never subscribed to the
      // snapshot (it does so only for a held session), and the draft pane's dock and merged preview
      // stayed invisible until a reload (D-144). Author-driven changes were already covered:
      // `invalidateSession` in `hooks/sessions.ts` invalidates the same chapter key.
      //
      // Scoped to the chapter the payload names: conception, research and canon-seed sessions have
      // no `chapter_id` and must not invalidate a chapter. `session.updated` carries the whole
      // `SessionDto`; `session.tool_call` carries only `{session_id, tool_call}`, so for it this is a
      // no-op — the server publishes a `session.updated` in the same batch as every tool call
      // (spec/10 §3), and that event carries the chapter.
      const chapterId = session?.chapter_id
      if (typeof chapterId === "number") {
        void queryClient.invalidateQueries({
          queryKey: keys.chapters.detail(chapterId),
        })
      }
      return
    }
    case "knowledge.changed": {
      // A scope or entry change: the knowledge pages and the hybrid search refetch (spec/10 §3).
      void queryClient.invalidateQueries({ queryKey: keys.knowledge.all })
      return
    }
    case "research.updated": {
      // A research task changed (create, edit, start, finish, defer): refetch the task and its
      // list (spec/10 §3). The overview follows too, because `next_action` and `attention` count
      // the book's `ready` tasks (D-136) and an agent or a second tab can defer one — without
      // this the card kept offering `run_research` after the last runnable task was deferred
      // (D-138).
      void queryClient.invalidateQueries({ queryKey: keys.research.all })
      const task = data?.task
      const bookId =
        typeof task === "object" && task !== null
          ? (task as Record<string, unknown>).book_id
          : undefined
      if (typeof bookId === "number") {
        void queryClient.invalidateQueries({
          queryKey: keys.books.overview(bookId),
        })
      }
      return
    }
    case "index.status": {
      // The settings page and the health badge show the embedder state (spec/10 §3).
      void queryClient.invalidateQueries({ queryKey: keys.embedding.status })
      return
    }
    case "resync": {
      void queryClient.invalidateQueries()
      return
    }
    default:
      return
  }
}
