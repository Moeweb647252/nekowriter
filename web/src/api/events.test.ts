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
