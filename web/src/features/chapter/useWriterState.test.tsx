import { renderHook } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"

vi.mock("@/api/hooks/sessions", () => ({
  useSession: () => ({ data: undefined }),
}))

import { useWriterState } from "@/features/chapter/useWriterState"

const planner = {
  id: 9,
  book_id: 1,
  role: "planner",
  kind: "chapter_outline",
  chapter_id: 7,
  status: "running",
} as never

describe("useWriterState locking", () => {
  it("locks the draft pane while a planner is running", () => {
    const chapter = { id: 7, open_sessions: [planner] } as unknown as ChapterDto
    const { result } = renderHook(() => useWriterState(chapter, undefined))
    expect(result.current.session?.id).toBe(9)
    expect(result.current.locked).toBe(true)
  })
})
