/** Full-text editor chapter identity and lock handling (P-01, P-02). */
import { useState } from "react"
import { act, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import { useChapterDoc } from "@/stores/chapterDoc"

const contentMutations = vi.hoisted(() => ({
  patchMutate: vi.fn(),
  putMutate: vi.fn(),
}))
vi.mock("@/api/hooks/chapters", () => ({
  useContentMutations: () => ({
    patch: { mutate: contentMutations.patchMutate, isPending: false },
    putText: { mutate: contentMutations.putMutate, isPending: false },
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

function chapter(id: number, text: string, contentVersion = 1): ChapterDto {
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
    content_version: contentVersion,
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
  contentMutations.patchMutate.mockClear()
  contentMutations.putMutate.mockClear()
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

  it("uses the chapter's own version after switching chapters (R8-01)", async () => {
    const a = chapter(1, "Chapter A text", 3)
    useChapterDoc.getState().load(1, 3, a.content)
    const b = chapter(2, "Chapter B text", 1)
    const { rerender } = render(view(a))
    rerender(view(b))
    act(() => {
      useChapterDoc.getState().load(2, 1, b.content)
    })
    expect(screen.queryByRole("status")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled()
  })

  it("does not close a full-text editor reopened while a save is in flight (S-4)", async () => {
    const user = userEvent.setup()
    function Harness() {
      const [open, setOpen] = useState(true)
      return (
        <ChapterText
          chapter={chapter(1, "Chapter A text")}
          bookId={1}
          unspaced
          fullTextOpen={open}
          onFullTextOpenChange={setOpen}
        />
      )
    }
    render(<Harness />)
    await user.click(screen.getByRole("button", { name: "Save" }))
    const options = contentMutations.putMutate.mock.calls[0]?.[1] as { onSuccess?: () => void }
    await user.click(screen.getByRole("button", { name: "Cancel" }))
    await user.click(screen.getByRole("button", { name: "Full-text edit" }))
    expect(screen.getByRole("textbox")).toBeInTheDocument()
    act(() => {
      options.onSuccess?.()
    })
    expect(screen.getByRole("textbox")).toBeInTheDocument()
  })

  it("opens against the live store after a blur save advanced it (R11-03)", () => {
    useChapterDoc.getState().load(1, 2, chapter(1, "Chapter A text").content)
    render(view(chapter(1, "Chapter A text")))
    expect(screen.queryByRole("status")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled()
  })

  it("refuses to save when the document version advances under the editor (S-3)", async () => {
    const user = userEvent.setup()
    render(view(chapter(1, "Chapter A text")))
    act(() => {
      useChapterDoc.getState().applyOps(
        2,
        [{ op: "update", pid: "p1", text: "Agent edit" }],
        "agent",
      )
    })
    expect(await screen.findByRole("status")).toHaveTextContent(
      "The chapter changed while this editor was open",
    )
    const save = screen.getByRole("button", { name: "Save" })
    expect(save).toBeDisabled()
    await user.click(save)
    expect(contentMutations.putMutate).not.toHaveBeenCalled()
  })

  it("is read-only and cannot save while the chapter is locked (P-02)", () => {
    writer.locked = true
    render(view(chapter(1, "Chapter A text")))
    expect(screen.getByRole("textbox")).toBeDisabled()
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled()
  })
})
