/** ReviewTab must hand `chapter.outline_missing` to a real OutlineDialog (R2-03). */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import { ApiError } from "@/api/client"
import type { BookSettings } from "@/api/gen/BookSettings"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { ReviewReportDto } from "@/api/gen/ReviewReportDto"

const sendBackMutate = vi.fn()
const writeOutlineMutate = vi.fn()
const report = {
  id: 1,
  chapter_id: 1,
  content_version: 0,
  attempt: 1,
  verdict: "revise",
  summary: "Needs work",
  issues: [],
  feedback: "Please fix the opening.",
  user_comment: "",
  created_at: "2026-01-01T00:00:00Z",
} as unknown as ReviewReportDto
const chapter = {
  id: 1,
  book_id: 1,
  track_id: 1,
  number: 1,
  title: "Chapter One",
  status: "in_review",
  content_version: 0,
  guidance: "",
} as unknown as ChapterDto
const SETTINGS: BookSettings = {
  granularity: "paragraph",
  auto_pause: false,
  pause_after_outline: true,
  target_words_min: 3000,
  target_words_max: 4000,
  recent_summary_count: 5,
  recent_chapter_count: 2,
  midterm_window: 10,
  auto_maintenance: true,
}

vi.mock("@/api/hooks/reviews", () => ({
  useReviews: () => ({ data: { reports: [report], feedback: [] }, isPending: false, isError: false }),
  useStartReview: () => ({ mutate: vi.fn(), isPending: false }),
  useRereview: () => ({ mutate: vi.fn(), isPending: false }),
  useSendBack: () => ({ mutate: sendBackMutate, isPending: false }),
}))
vi.mock("@/api/hooks/chapters", () => ({
  useWriteOutlineChapter: () => ({ mutate: writeOutlineMutate, isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import { ReviewTab } from "@/features/chapter/ReviewTab"

beforeEach(() => sendBackMutate.mockReset())

describe("ReviewTab outline-missing handoff", () => {
  it("closes Discuss when the chapter changes", async () => {
    const user = userEvent.setup()
    const view = (current: ChapterDto) => (
      <ReviewTab chapter={current} bookId={1} settings={SETTINGS} onSessionStarted={() => {}} />
    )
    const { rerender } = render(view(chapter))
    await user.click(screen.getAllByText("Discuss…")[0].closest("button") as HTMLElement)
    expect(screen.getByText("Discuss with the reviewer")).toBeInTheDocument()
    rerender(view({ ...chapter, id: 2, number: 2 }))
    expect(screen.queryByText("Discuss with the reviewer")).not.toBeInTheDocument()
  })

  it("closes Send back when the chapter changes", async () => {
    const user = userEvent.setup()
    const view = (current: ChapterDto) => (
      <ReviewTab chapter={current} bookId={1} settings={SETTINGS} onSessionStarted={() => {}} />
    )
    const { rerender } = render(view(chapter))
    await user.click(screen.getByRole("button", { name: "Send back with this feedback" }))
    expect(screen.getByText("Send back for revision")).toBeInTheDocument()
    rerender(view({ ...chapter, id: 2, number: 2 }))
    expect(screen.queryByText("Send back for revision")).not.toBeInTheDocument()
  })

  it("opens the Outline dialog from the send-back dialog", async () => {
    const user = userEvent.setup()
    sendBackMutate.mockImplementation((...args: unknown[]) => {
      const options = args[1] as { onError?: (error: unknown) => void }
      options?.onError?.(new ApiError(409, "chapter.outline_missing", "missing"))
    })
    render(
      <ReviewTab
        chapter={chapter}
        bookId={1}
        settings={SETTINGS}
        onSessionStarted={() => {}}
      />,
    )
    await user.click(screen.getByRole("button", { name: "Send back with this feedback" }))
    await user.click(screen.getByRole("button", { name: "Send back" }))
    expect(screen.getByText("This chapter has no outline")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Write the outline" }))
    expect(screen.getByText("Write the chapter outline")).toBeInTheDocument()
  })
})
