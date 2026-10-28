/** Full-text editor chapter identity and lock handling (P-01, P-02). */
import { render, screen } from "@testing-library/react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import { useChapterDoc } from "@/stores/chapterDoc"

vi.mock("@/api/hooks/chapters", () => ({
  useContentMutations: () => ({
    patch: { mutate: vi.fn(), isPending: false },
    putText: { mutate: vi.fn(), isPending: false },
  }),
}))
const writer = vi.hoisted(() => ({ locked: false }))
vi.mock("@/features/chapter/useWriterState", () => ({
  useWriterState: () => ({ locked: writer.locked }),
  openWriterSession: () => undefined,
  heldPids: () => [],
}))
vi.mock("@/api/hooks/sessions", () => ({
  useResumeSession: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => () => {},
  useWarningToasts: () => () => {},
}))

import { ChapterText } from "@/features/chapter/ChapterText"

function chapter(id: number, text: string): ChapterDto {
  return {
    id,
    book_id: 1,
    track_id: 1,
    number: id,
    title: `Chapter ${id}`,
    status: "drafting",
    canon_status: "none",
    word_count: 2,
    has_outline: false,
    content_version: 1,
    content: { paragraphs: [{ pid: "p1", text }], next_pid: 2, tail_open: false },
    open_sessions: [],
    guidance: "",
    outline: "",
    reference_notes: "",
    summary: "",
    official_version_id: null,
    updated_at: "2026-01-01T00:00:00Z",
  }
}

beforeEach(() => {
  writer.locked = false
  useChapterDoc.getState().load(1, 1, chapter(1, "A").content)
})

function view(current: ChapterDto) {
  return (
    <ChapterText
      chapter={current}
      bookId={1}
      unspaced
      fullTextOpen
      onFullTextOpenChange={() => {}}
    />
  )
}

describe("FullTextEditor", () => {
  it("adopts the next chapter's text after the chapter changes (P-01)", () => {
    const { rerender } = render(view(chapter(1, "Chapter A text")))
    expect(screen.getByRole("textbox")).toHaveValue("Chapter A text")
    rerender(view(chapter(2, "Chapter B text")))
    expect(screen.getByRole("textbox")).toHaveValue("Chapter B text")
  })

  it("is read-only and cannot save while the chapter is locked (P-02)", () => {
    writer.locked = true
    render(view(chapter(1, "Chapter A text")))
    expect(screen.getByRole("textbox")).toBeDisabled()
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled()
  })
})
