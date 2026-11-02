/** The cross-canon merge source union (spec/06 §9, D-161 18.3-S1). */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { renderHook, waitFor } from "@testing-library/react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { TrackDto } from "@/api/gen/TrackDto"

const listEntities = vi.hoisted(() => vi.fn())

vi.mock("@/api/endpoints/canon", () => ({
  listEntities: (...args: unknown[]) => listEntities(...args),
}))

vi.mock("@/api/hooks/tracks", () => ({
  useTracks: () => ({
    isPending: false,
    data: [
      { id: 1, kind: "main", title: "", canon_mode: "canon" },
      { id: 2, kind: "side", title: "Isolated", canon_mode: "isolated" },
      { id: 3, kind: "side", title: "Shared side", canon_mode: "canon" },
    ] as unknown as TrackDto[],
  }),
}))

import { useMergeSourceEntities } from "@/api/hooks/useCanon"

function wrapper({ children }: { children: React.ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>
}

beforeEach(() => {
  listEntities.mockReset()
  listEntities.mockImplementation(
    async (_bookId: number, params?: { trackId?: number }) => {
      if (params?.trackId === 2) {
        return [
          { id: 7, track_id: null, kind: "character", name: "Shared" },
          { id: 8, track_id: 2, kind: "item", name: "Private" },
        ]
      }
      return [{ id: 7, track_id: null, kind: "character", name: "Shared" }]
    },
  )
})

describe("useMergeSourceEntities", () => {
  it("unions the shared list with every isolated track", async () => {
    const { result } = renderHook(() => useMergeSourceEntities(1), { wrapper })
    await waitFor(() => expect(result.current.data).toHaveLength(2))
    expect(result.current.data.map((entity) => entity.name)).toEqual([
      "Private",
      "Shared",
    ])
    expect(listEntities).toHaveBeenCalledWith(1)
    expect(listEntities).toHaveBeenCalledWith(1, { trackId: 2 })
    // A canon-mode side track's rows are shared and the shared query already returned them.
    expect(listEntities).not.toHaveBeenCalledWith(1, { trackId: 3 })
    // The shared row appears once even though both queries return it.
    expect(result.current.data.filter((entity) => entity.id === 7)).toHaveLength(1)
  })
})
