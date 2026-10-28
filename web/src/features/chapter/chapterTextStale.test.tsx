/** The paragraph editor refuses an advanced store version (R8-02). */
import { act, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import { useChapterDoc } from "@/stores/chapterDoc"

const mutations = vi.hoisted(() => ({ patchMutate: vi.fn(), putMutate: vi.fn() }))
vi.mock("@/api/hooks/chapters", () => ({
  useContentMutations: () => ({
    patch: { mutate: mutations.patchMutate, isPending: false },
    putText: { mutate: mutations.putMutate, isPending: false },
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

function chapter(): ChapterDto {
  return {
    id: 1,
    book_id: 1,
    track_id: 1,
    number: 1,
    title: "Chapter",
    status: "drafting",
    canon_status: "none",
    word_count: 1,
    has_outline: false,
    content_version: 1,
    content: { paragraphs: [{ pid: "p1", text: "A 的正文" }], next_pid: 2, tail_open: false },
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
  mutations.patchMutate.mockClear()
  mutations.putMutate.mockClear()
  useChapterDoc.getState().load(1, 1, chapter().content)
})

describe("chapter paragraph stale version", () => {
  it("refuses to save a paragraph after the document version advances", async () => {
    const user = userEvent.setup()
    render(
      <ChapterText
        chapter={chapter()}
        bookId={1}
        unspaced
        fullTextOpen={false}
        onFullTextOpenChange={() => {}}
      />,
    )
    await user.click(screen.getByText("A 的正文"))
    const editor = screen.getByRole("textbox")
    await user.clear(editor)
    await user.type(editor, "A 的编辑")
    act(() => {
      useChapterDoc.getState().applyOps(
        2,
        [{ op: "update", pid: "p1", text: "Agent 的正文" }],
        "agent",
      )
    })
    await user.tab()
    expect(mutations.patchMutate).not.toHaveBeenCalled()
    expect(await screen.findByRole("status")).toHaveTextContent(
      "The chapter changed while this editor was open",
    )
  })
})
