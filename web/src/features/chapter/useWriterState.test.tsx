import { renderHook } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { SessionDto } from "@/api/gen/SessionDto"

let snapshot: unknown = undefined
vi.mock("@/api/hooks/sessions", () => ({
  useSession: () => ({ data: snapshot }),
}))

import { useWriterState } from "@/features/chapter/useWriterState"
import { useChapterDoc } from "@/stores/chapterDoc"
import { useLiveStore } from "@/stores/live"

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


const writer = {
  id: 11,
  book_id: 1,
  role: "writer",
  kind: "chapter_write",
  chapter_id: 7,
  status: "held",
  held_tool_call_id: 2,
} as unknown as SessionDto

describe("useWriterState previews and holds", () => {
  it("returns only a held call, not a done call left in held_tool_call_id", () => {
    snapshot = {
      tool_calls: [{ id: 2, status: "done" }],
      messages: [],
    }
    const chapter = { id: 7, open_sessions: [writer] } as unknown as ChapterDto
    const { result } = renderHook(() => useWriterState(chapter, undefined))
    expect(result.current.held).toBeUndefined()
  })

  it("returns a pinned preview only while its draft version is current", () => {
    snapshot = { tool_calls: [], messages: [] }
    useChapterDoc.setState({ version: 2 })
    useLiveStore.setState({
      previews: {
        "11:3:0": {
          name: "insert_text",
          field: "text",
          text: "pinned",
          pinned: { version: 2 },
        },
      },
    })
    const running = { ...writer, status: "running", held_tool_call_id: null } as SessionDto
    const chapter = { id: 7, open_sessions: [running] } as unknown as ChapterDto
    const { result, rerender } = renderHook(() => useWriterState(chapter, undefined))
    expect(result.current.preview?.text).toBe("pinned")
    useChapterDoc.setState({ version: 3 })
    rerender()
    expect(result.current.preview).toBeUndefined()
  })
})
