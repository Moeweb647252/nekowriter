/** A running planner locks the draft pane through the real useWriterState hook (R3-01). */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import { useChapterDoc } from "@/stores/chapterDoc"

vi.mock("@/api/hooks/chapters", () => ({
  useContentMutations: () => ({
    patch: { mutate: vi.fn(), isPending: false },
    putText: { mutate: vi.fn(), isPending: false },
  }),
}))
vi.mock("@/api/hooks/sessions", () => ({
  useSession: () => ({ data: undefined }),
  useResumeSession: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => () => {},
  useWarningToasts: () => () => {},
}))

import { ChapterText } from "@/features/chapter/ChapterText"

const CHAPTER = {
  id: 2,
  book_id: 3,
  track_id: 1,
  number: 1,
  title: "Chapter",
  status: "planned",
  canon_status: "none",
  word_count: 3,
  has_outline: false,
  content_version: 1,
  content: { paragraphs: [{ pid: "p1", text: "第一段。" }], next_pid: 2, tail_open: false },
  open_sessions: [
    {
      id: 9,
      book_id: 3,
      role: "planner",
      kind: "chapter_outline",
      chapter_id: 2,
      status: "running",
      phase: "none",
    },
  ],
  guidance: "",
  outline: "",
  reference_notes: "",
  summary: "",
  official_version_id: null,
  updated_at: "2026-01-01T00:00:00Z",
} as unknown as ChapterDto

beforeEach(() => {
  useChapterDoc.getState().load(CHAPTER.id, CHAPTER.content_version, CHAPTER.content)
})

describe("ChapterText planner lock", () => {
  it("renders read-only while the planner runs", async () => {
    const user = userEvent.setup()
    render(
      <ChapterText
        chapter={CHAPTER}
        bookId={3}
        unspaced
        fullTextOpen={false}
        onFullTextOpenChange={() => {}}
      />,
    )
    expect(screen.getByRole("status")).toHaveTextContent(
      "The writer is working — pause to edit · 3 words in this version",
    )
    await user.click(screen.getByText("第一段。"))
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument()
  })
})
