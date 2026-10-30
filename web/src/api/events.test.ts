/**
 * The SSE dispatch table (spec/11 §4.2, spec/10 §3).
 *
 * The server listens for every event type, but a type without a `case` in `applyServerEvent` is
 * silently dropped: the canon pages did not react to an archivist's maintenance writes and the
 * proposals page/badge did not react to a proposal created or decided in an agent session.
 */
import { QueryClient } from "@tanstack/react-query"
import { describe, expect, it } from "vitest"

import { applyServerEvent, flushLiveDeltas } from "@/api/events"
import { keys } from "@/api/keys"
import { useChapterDoc } from "@/stores/chapterDoc"
import { useLiveStore } from "@/stores/live"

function isInvalidated(
  client: QueryClient,
  queryKey: readonly unknown[]
): boolean {
  const query = client.getQueryCache().find({ queryKey })
  return query?.state.isInvalidated ?? false
}

/** Seeds one query per cache the M6 events touch, plus an unrelated chapter. */
function seededClient(): QueryClient {
  const client = new QueryClient()
  client.setQueryData(keys.canon.entities(1), [])
  client.setQueryData(keys.canon.memory(1), {})
  client.setQueryData(keys.changes.all, [])
  client.setQueryData(keys.books.overview(1), {})
  client.setQueryData(keys.proposals.list(1, "pending"), [])
  client.setQueryData(keys.proposals.list(1), [])
  client.setQueryData(keys.chapters.detail(9), {})
  client.setQueryData(keys.tracks.chapters(3), [])
  client.setQueryData(keys.canon.goals(1, "shared|all|||"), [])
  return client
}

describe("applyServerEvent", () => {
  it("invalidates the canon caches and the overview on canon.changed", () => {
    const client = seededClient()
    applyServerEvent(client, "canon.changed", {
      book_id: 1,
      target_type: "entity",
      target_id: 2,
      op: "update",
      change_id: 3,
    })
    expect(isInvalidated(client, keys.canon.entities(1))).toBe(true)
    expect(isInvalidated(client, keys.canon.memory(1))).toBe(true)
    expect(isInvalidated(client, keys.changes.all)).toBe(true)
    expect(isInvalidated(client, keys.books.overview(1))).toBe(true)
    expect(isInvalidated(client, keys.chapters.detail(9))).toBe(false)
  })

  it("keeps next_pid in step with chapter.content events", () => {
    const client = seededClient()
    const content = {
      paragraphs: [{ pid: "p1", text: "one" }],
      next_pid: 2,
      tail_open: false,
    }
    client.setQueryData(keys.chapters.detail(9), {
      id: 9,
      content,
      content_version: 1,
      word_count: 1,
    })
    useChapterDoc.getState().load(9, 1, content)
    applyServerEvent(client, "chapter.content", {
      chapter_id: 9,
      version: 2,
      ops: [{ op: "insert", after: "p1", pid: "p5", text: "external" }],
      word_count: 2,
      tail_open: false,
      actor: "agent",
    })
    expect(useChapterDoc.getState().nextPid).toBe(6)
    expect(client.getQueryData<{ content: { next_pid: number } }>(keys.chapters.detail(9))?.content.next_pid).toBe(6)
  })

  it("invalidates the proposal lists and the overview on proposal.changed", () => {
    const client = seededClient()
    applyServerEvent(client, "proposal.changed", {
      book_id: 1,
      proposal_id: 7,
      status: "pending",
    })
    expect(isInvalidated(client, keys.proposals.list(1, "pending"))).toBe(true)
    expect(isInvalidated(client, keys.proposals.list(1, undefined))).toBe(true)
    expect(isInvalidated(client, keys.books.overview(1))).toBe(true)
    expect(isInvalidated(client, keys.chapters.detail(9))).toBe(false)
  })

  it("invalidates goal lists on chapter.updated because approval can make a chapter_at_least condition ready", () => {
    const client = seededClient()
    applyServerEvent(client, "chapter.updated", {
      chapter: {
        id: 44,
        book_id: 1,
        track_id: 3,
        number: 12,
        title: "Approved",
        status: "official",
        canon_status: "synced",
        word_count: 100,
        has_outline: true,
        updated_at: "2026-01-01T00:00:00Z",
      },
      outline_changed: false,
    })
    expect(isInvalidated(client, keys.canon.goals(1, "shared|all|||"))).toBe(true)
    expect(isInvalidated(client, keys.books.overview(1))).toBe(true)
    expect(isInvalidated(client, keys.tracks.chapters(3))).toBe(true)
  })
})

describe("delta coalescing", () => {
  it("batches deltas into one frame and merges the same stream", () => {
    useLiveStore.setState({ streams: {}, previews: {} })
    const client = new QueryClient()
    applyServerEvent(client, "session.delta", {
      session_id: 1,
      seq: 3,
      kind: "text",
      index: 0,
      text: "Hel",
    })
    applyServerEvent(client, "session.delta", {
      session_id: 1,
      seq: 3,
      kind: "text",
      index: 0,
      text: "lo",
    })
    // Nothing reaches the store before the frame: a streaming turn does not re-render per byte.
    expect(useLiveStore.getState().streams[1]).toBeUndefined()
    flushLiveDeltas()
    expect(useLiveStore.getState().streams[1]?.text[0]).toBe("Hello")
    useLiveStore.setState({ streams: {}, previews: {} })
  })

  it("invalidates the overview on research.updated so a defer unblocks writing", () => {
    // The overview's `next_action` counts `ready` research tasks (D-136, D-138), so deferring one
    // changes it. Without this invalidation the card kept offering `run_research` after the author
    // deferred the last runnable task, and the release valve looked broken until a manual reload.
    const client = seededClient()
    client.setQueryData(keys.research.list(1), [])
    applyServerEvent(client, "research.updated", {
      task: {
        id: 3,
        book_id: 1,
        scope_name: "明初历史",
        topic: "宝钞与财政",
        requirements: "",
        language: "zh-CN",
        status: "deferred",
        report: "",
        entries_created: 0,
        entries: [],
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-02T00:00:00Z",
      },
    })

    expect(isInvalidated(client, keys.research.list(1))).toBe(true)
    expect(isInvalidated(client, keys.books.overview(1))).toBe(true)
  })

  it("invalidates the chapter a session.updated names, so an agent-driven hold reaches the dock", () => {
    // `ChapterDto.open_sessions` is the client's only handle on the chapter's writer session, and
    // `useWriterState` subscribes to the session snapshot only while that session is `held`. When
    // the agent holds (`auto_pause`, `pause_after_outline`) no client mutation runs, so the chapter
    // query has to be refreshed by the event; without it the cached session stayed `running`, the
    // dock and the merged preview never appeared, and a reload "fixed" it (D-144).
    const client = seededClient()
    applyServerEvent(client, "session.updated", {
      session: {
        id: 11,
        book_id: 1,
        role: "writer",
        kind: "chapter_write",
        chapter_id: 9,
        status: "held",
        held_tool_call_id: 317,
      },
    })

    expect(isInvalidated(client, keys.chapters.detail(9))).toBe(true)
  })

  it("does not invalidate a chapter for a session.updated without a chapter", () => {
    // Conception, research and canon-seed sessions are not per-chapter (spec/10 §2.8). Their events
    // must not invalidate a chapter, or every research step would refetch the open chapter.
    const client = seededClient()
    applyServerEvent(client, "session.updated", {
      session: {
        id: 7,
        book_id: 1,
        role: "conception",
        kind: "conception",
        chapter_id: null,
        status: "running",
      },
    })

    expect(isInvalidated(client, keys.chapters.detail(9))).toBe(false)
  })

  it("drops buffered deltas when the persisted message arrives", () => {
    useLiveStore.setState({ streams: {}, previews: {} })
    const client = new QueryClient()
    applyServerEvent(client, "session.delta", {
      session_id: 1,
      seq: 3,
      kind: "text",
      index: 0,
      text: "stale",
    })
    applyServerEvent(client, "session.message", {
      session_id: 1,
      message: { id: 9, seq: 3 },
    })
    flushLiveDeltas()
    expect(useLiveStore.getState().streams[1]).toBeUndefined()
  })
})
