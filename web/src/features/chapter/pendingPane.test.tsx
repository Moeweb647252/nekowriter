/**
 * The draft pane showing a held chapter write (PROJECT §8.5, spec/11 §5.2, D-141).
 *
 * The author's report was that the body is written in the middle draft pane — so the pane has to
 * show what the agent wrote *as the draft would read*, and the decision has to be taken from there.
 * This covers the pane half:
 *
 * - the merged document replaces the editable paragraphs (the staged text is not the draft's text,
 *   so it must not be click-to-editable, and the full-text editor must not open on it);
 * - the paragraphs the write adds or changes carry the waiting tint, and a note says where the
 *   text came from, so the tint is never the only signal;
 * - the word line names **both** counts (the confirmed one the header shows, and the projection),
 *   rather than silently replacing the header's number with a different one;
 * - the inline `HoldBar` is gone for a write hold, because the dock now owns that decision.
 *
 * Fail-before: before this change the pane rendered `chapterDoc.paragraphs` only, so a held write
 * was invisible there (its text existed only as a live ghost while streaming, and as the panel's
 * raw input afterwards) and `HoldBar` offered Continue/Feedback for it under the paragraphs.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import type { PendingDraftView } from "@/features/chapter/pendingWrite"
import { useChapterDoc } from "@/stores/chapterDoc"

vi.mock("@/api/hooks/chapters", () => ({
  useContentMutations: () => ({
    patch: { mutate: vi.fn(), isPending: false },
    putText: { mutate: vi.fn(), isPending: false },
  }),
}))

/** The writer state the pane reads; each test sets it. */
const writer = vi.hoisted(() => ({
  session: undefined as unknown,
  locked: false,
  held: undefined as unknown,
  heldPids: [] as string[],
  heldOutline: false,
  heldWrite: false,
  pendingWrite: undefined as unknown,
  preview: undefined as unknown,
}))

vi.mock("@/features/chapter/useWriterState", () => ({
  useWriterState: () => writer,
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
import { PendingText } from "@/features/chapter/PendingText"

const CHAPTER: ChapterDto = {
  id: 2,
  book_id: 3,
  track_id: 1,
  number: 1,
  title: "醒来已是洪武元年",
  status: "drafting",
  canon_status: "none",
  word_count: 6,
  has_outline: false,
  content_version: 4,
  content: {
    paragraphs: [
      { pid: "p1", text: "第一段。" },
      { pid: "p2", text: "第二段。" },
    ],
    next_pid: 3,
    tail_open: false,
  },
  open_sessions: [],
  guidance: "",
  outline: "",
  reference_notes: "",
  summary: "",
  official_version_id: null,
  updated_at: "2026-01-01T00:00:00Z",
}

/** The merged view `ChapterPage` derives and passes in. */
const VIEW: PendingDraftView = {
  standing: "current",
  expectedVersion: 4,
  draftVersion: 4,
  paragraphs: [
    { pid: "p1", text: "第一段。", kind: "confirmed" },
    { pid: "p2", text: "改写后的第二段。", kind: "changed" },
    { pid: "p3", text: "智能体新写的一段。", kind: "added" },
  ],
  stagedWords: 20,
  confirmedWords: 6,
}

function renderPane(pendingView?: PendingDraftView) {
  return render(
    <ChapterText
      chapter={CHAPTER}
      bookId={3}
      unspaced
      fullTextOpen={false}
      onFullTextOpenChange={() => {}}
      pendingView={pendingView}
    />,
  )
}

beforeEach(() => {
  writer.locked = false
  writer.held = undefined
  writer.heldWrite = false
  writer.heldPids = []
  // The store is what the pane reads for the document on screen; each test loads the confirmed
  // draft as `ChapterPage` would.
  useChapterDoc.getState().load(CHAPTER.id, CHAPTER.content_version, CHAPTER.content)
})

describe("PendingText", () => {
  it("renders every paragraph of the merged document with its origin", () => {
    render(<PendingText paragraphs={VIEW.paragraphs} unspaced />)
    const first = screen.getByText("第一段。")
    expect(first).toHaveAttribute("data-kind", "confirmed")
    expect(screen.getByText("改写后的第二段。")).toHaveAttribute("data-kind", "changed")
    expect(screen.getByText("智能体新写的一段。")).toHaveAttribute("data-kind", "added")
  })

  it("renders read-only while the chapter's owner is working", async () => {
    writer.locked = true
    renderPane()
    expect(screen.getByText("The agent is working — Pause to edit.")).toBeInTheDocument()
    await userEvent.click(screen.getByText("第一段。"))
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument()
  })

  it("clears HoldBar feedback when the chapter changes", async () => {
    const user = userEvent.setup()
    writer.held = { name: "insert_text" }
    writer.heldWrite = false
    writer.heldPids = ["p1"]
    const view = (chapterValue: ChapterDto) => (
      <ChapterText
        chapter={chapterValue}
        bookId={3}
        unspaced
        fullTextOpen={false}
        onFullTextOpenChange={() => {}}
      />
    )
    const second = { ...CHAPTER, id: 99, number: 9, title: "Second" }
    const { rerender } = render(view(CHAPTER))
    await user.click(screen.getByRole("button", { name: "Feedback…" }))
    await user.type(screen.getByRole("textbox"), "A 的反馈")
    rerender(view(second))
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument()
  })

  it("is editable again in the next test, not stuck on the previous lock", async () => {
    renderPane()
    await userEvent.click(screen.getByText("第一段。"))
    expect(screen.getByRole("textbox")).toBeInTheDocument()
  })

  it("does not offer the paragraph editor", () => {
    // The merged text is not the draft's; editing it would save a paragraph the author is not
    // looking at and bump the version the staged write is compared against.
    render(<PendingText paragraphs={VIEW.paragraphs} unspaced />)
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument()
  })
})

describe("ChapterText with a held write", () => {
  it("renders the merged document instead of the editable one", () => {
    renderPane(VIEW)
    // The staged paragraph is on screen…
    expect(screen.getByText("智能体新写的一段。")).toBeInTheDocument()
    // …the rewritten one shows the new text, not the confirmed text…
    expect(screen.getByText("改写后的第二段。")).toBeInTheDocument()
    expect(screen.queryByText("第二段。")).not.toBeInTheDocument()
    // …and the way into the full-text editor is closed, because that editor would edit the draft
    // rather than the merged text on screen.
    expect(screen.getByRole("button", { name: "Full-text edit" })).toBeDisabled()
  })

  it("names both word counts, so the header's number is not contradicted", () => {
    // The header counts what the chapter holds (spec/11 §8); the pane states the same number and
    // adds the projection, instead of replacing it with the staged count.
    renderPane(VIEW)
    expect(screen.getByText("20 words once applied (6 now)")).toBeInTheDocument()
  })

  it("keeps the plain count when there is no staged write", () => {
    renderPane(undefined)
    expect(screen.getByText("6 words in this version")).toBeInTheDocument()
    expect(screen.queryByText(/once applied/)).not.toBeInTheDocument()
  })

  it("leaves the inline bar to the non-write holds", () => {
    // A held write is decided in the dock, so the bar must not offer Continue/Feedback for it as
    // well. Here the writer is held on a NON-write tool, which keeps the bar.
    writer.held = { id: 300, name: "search_canon", ui: { pid: "p2" } } as unknown as ToolCallDto
    writer.heldPids = ["p2"]
    renderPane(undefined)
    expect(screen.getByRole("group", { name: "Paused after this change (p2)" })).toBeInTheDocument()
  })

  it("does not render the bar for a held chapter write", () => {
    writer.held = { id: 283, name: "insert_text", ui: {} } as unknown as ToolCallDto
    writer.heldWrite = true
    renderPane(VIEW)
    expect(screen.queryByRole("group")).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Continue" })).not.toBeInTheDocument()
  })
})
