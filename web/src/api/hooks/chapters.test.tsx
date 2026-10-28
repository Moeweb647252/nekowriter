/** Late content/restore responses must not write into another open chapter (R7-01, R7-02). */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"

const endpoints = vi.hoisted(() => ({
  patchContent: vi.fn(),
  putContentText: vi.fn(),
  restoreVersion: vi.fn(),
}))
vi.mock("@/api/endpoints/chapters", () => endpoints)

import { useContentMutations, useRestoreVersion } from "@/api/hooks/chapters"
import { useChapterDoc } from "@/stores/chapterDoc"

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>
}

function paragraph(pid: string, text: string) {
  return { pid, text }
}

function chapter(id: number, text: string): ChapterDto {
  return {
    id,
    book_id: 1,
    track_id: 1,
    number: id,
    title: "Chapter",
    status: "drafting",
    canon_status: "none",
    word_count: 1,
    has_outline: false,
    content_version: 1,
    content: { paragraphs: [paragraph("p1", text)], next_pid: 2, tail_open: false },
    open_sessions: [],
    guidance: "",
    outline: "",
    reference_notes: "",
    summary: "",
    official_version_id: null,
    updated_at: "2026-01-01T00:00:00Z",
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

beforeEach(() => {
  vi.clearAllMocks()
  useChapterDoc.getState().reset()
})

describe("chapter content mutation response routing", () => {
  it("does not apply chapter A's late patch response to chapter B's document", async () => {
    const pending = deferred<{ version: number; ops: { op: "update"; pid: string; text: string }[]; word_count: number; tail_open: boolean }>()
    endpoints.patchContent.mockReturnValue(pending.promise)
    useChapterDoc.getState().load(1, 1, { paragraphs: [paragraph("p1", "A 的正文")], next_pid: 2, tail_open: false })
    const { result } = renderHook(() => useContentMutations(1, 1), { wrapper })
    act(() => {
      result.current.patch.mutate({ base_version: 1, ops: [{ op: "update", pid: "p1", text: "A edit" }] })
    })
    await waitFor(() => expect(endpoints.patchContent).toHaveBeenCalledTimes(1))
    useChapterDoc.getState().load(2, 1, { paragraphs: [paragraph("p1", "B 的正文")], next_pid: 2, tail_open: false })
    await act(async () => {
      pending.resolve({
        version: 2,
        ops: [{ op: "update", pid: "p1", text: "A edit" }],
        word_count: 1,
        tail_open: false,
      })
    })
    await waitFor(() => expect(useChapterDoc.getState().chapterId).toBe(2))
    expect(useChapterDoc.getState().paragraphs[0]?.text).toBe("B 的正文")
  })

  it("does not load chapter A's late restore response into chapter B's open document", async () => {
    const pending = deferred<ChapterDto>()
    endpoints.restoreVersion.mockReturnValue(pending.promise)
    useChapterDoc.getState().load(2, 1, { paragraphs: [paragraph("p1", "B 的正文")], next_pid: 2, tail_open: false })
    const { result } = renderHook(() => useRestoreVersion(1), { wrapper })
    act(() => {
      result.current.mutate(11)
    })
    await waitFor(() => expect(endpoints.restoreVersion).toHaveBeenCalledWith(11))
    await act(async () => {
      pending.resolve(chapter(1, "A 的恢复正文"))
    })
    await waitFor(() => expect(useChapterDoc.getState().chapterId).toBe(2))
    expect(useChapterDoc.getState().paragraphs[0]?.text).toBe("B 的正文")
  })
})
