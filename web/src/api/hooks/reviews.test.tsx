/**
 * Review mutations invalidate the query keys they actually change (spec/11 §4.1).
 *
 * A send-back moves the chapter to `drafting`/`revising` as a session-creation side effect, which
 * emits no `chapter.updated` event; the navigator's chapter rows live under the *track* key, so
 * invalidating `keys.tracks.chapters(chapterId)` (the chapter id) refreshed nothing.
 */
import type { ReactNode } from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

vi.mock("@/api/endpoints/reviews", () => ({
  listReviews: vi.fn(),
  startReview: vi.fn(),
  rereview: vi.fn(),
  sendBack: vi.fn(async () => ({ session_id: 9 })),
  maintainChapter: vi.fn(),
  resyncChapter: vi.fn(),
  listProposals: vi.fn(),
  acceptProposal: vi.fn(),
  rejectProposal: vi.fn(),
}))

import { keys } from "@/api/keys"
import { useSendBack } from "@/api/hooks/reviews"

function wrapperFor(client: QueryClient) {
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

describe("useSendBack", () => {
  it("invalidates the chapter's track list, not a key built from the chapter id", async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    const chapterId = 5
    const trackId = 2
    const bookId = 1
    // The chapter detail is cached by the page that shows the action; the navigator holds the
    // track's chapter list.
    client.setQueryData(keys.chapters.detail(chapterId), { track_id: trackId })
    client.setQueryData(keys.tracks.chapters(trackId), [])
    client.setQueryData(keys.tracks.chapters(chapterId), [])

    const { result } = renderHook(() => useSendBack(chapterId, bookId), {
      wrapper: wrapperFor(client),
    })
    await act(async () => {
      await result.current.mutateAsync({
        feedback: "fix it",
        mode: "revise",
        issue_indexes: [],
      })
    })

    await waitFor(() => {
      const track = client
        .getQueryCache()
        .find({ queryKey: keys.tracks.chapters(trackId) })
      expect(track?.state.isInvalidated).toBe(true)
    })
    const wrongKey = client
      .getQueryCache()
      .find({ queryKey: keys.tracks.chapters(chapterId) })
    expect(wrongKey?.state.isInvalidated).toBe(false)
  })
})
