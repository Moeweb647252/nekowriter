/**
 * The SSE dispatch table (spec/11 §4.2, spec/10 §3).
 *
 * The server listens for every event type, but a type without a `case` in `applyServerEvent` is
 * silently dropped: the canon pages did not react to an archivist's maintenance writes and the
 * proposals page/badge did not react to a proposal created or decided in an agent session.
 */
import { QueryClient } from "@tanstack/react-query"
import { describe, expect, it } from "vitest"

import { applyServerEvent } from "@/api/events"
import { keys } from "@/api/keys"

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
