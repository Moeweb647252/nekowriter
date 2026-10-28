/**
 * Submitting for review warns before it ends an open writer session (spec/08 §1.1, task-20).
 *
 * The spec's Submit row ends an open, non-running writer session; the backend now does that, so
 * the button must ask first instead of silently killing the session the author paused.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

const submitMutate = vi.fn()

vi.mock("@/api/hooks/chapters", () => ({
  useSubmitChapter: () => ({ mutate: submitMutate, isPending: false }),
  useApproveChapter: () => ({ mutate: vi.fn(), isPending: false }),
  useWriteOutlineChapter: () => ({ mutate: vi.fn(), isPending: false }),
}))

vi.mock("@/api/hooks/reviews", () => ({
  useReviews: () => ({ data: { reports: [], feedback: [] } }),
  useStartReview: () => ({ mutate: vi.fn(), isPending: false }),
  useMaintainChapter: () => ({ mutate: vi.fn(), isPending: false }),
  useResyncChapter: () => ({ mutate: vi.fn(), isPending: false }),
}))

vi.mock("@/features/chapter/WriteDialog", () => ({ WriteDialog: () => null }))
vi.mock("@/features/chapter/SendBackDialog", () => ({
  SendBackDialog: () => null,
}))
vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => () => {},
  useWarningToasts: () => () => {},
}))

import { ChapterHeader } from "@/features/chapter/ChapterHeader"
import type { BookSettings } from "@/api/gen/BookSettings"
import type { ChapterDto } from "@/api/gen/ChapterDto"

const SETTINGS: BookSettings = {
  granularity: "paragraph",
  auto_pause: false,
  pause_after_outline: true,
  target_words_min: 3000,
  target_words_max: 4000,
  recent_summary_count: 5,
  midterm_window: 10,
  auto_maintenance: true,
}

/** A drafting chapter with one paragraph and the given open sessions. */
function chapter(openSessions: unknown[]): ChapterDto {
  return {
    id: 4,
    book_id: 1,
    track_id: 2,
    number: 4,
    title: "Chapter Four",
    status: "drafting",
    canon_status: "none",
    word_count: 6,
    updated_at: "2026-01-01T00:00:00Z",
    guidance: "",
    outline: "",
    reference_notes: "",
    summary: "",
    content: {
      paragraphs: [{ pid: "p1", text: "第一段。" }],
      next_pid: 2,
      tail_open: false,
    },
    content_version: 3,
    open_sessions: openSessions,
  } as unknown as ChapterDto
}

function renderHeader(current: ChapterDto) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  render(
    <QueryClientProvider client={client}>
      <ChapterHeader
        chapter={current}
        settings={SETTINGS}
        language="en"
        outlineState="accepted"
        onRename={() => {}}
      />
    </QueryClientProvider>
  )
}

beforeEach(() => {
  submitMutate.mockClear()
})

describe("ChapterHeader submit", () => {
  it("warns before ending an open, non-running writer session", async () => {
    const user = userEvent.setup()
    renderHeader(chapter([{ role: "writer", status: "stopped" }]))

    await user.click(screen.getByRole("button", { name: "Submit for review" }))
    expect(
      submitMutate,
      "clicking submit alone must not end the paused writer session"
    ).not.toHaveBeenCalled()
    expect(screen.getByText("End the open session?")).toBeInTheDocument()

    await user.click(
      screen.getByRole("button", { name: "Submit and end the session" })
    )
    expect(submitMutate).toHaveBeenCalledTimes(1)
  })

  it("warns when a stopped planner session will be ended", async () => {
    const user = userEvent.setup()
    renderHeader(chapter([{ role: "planner", status: "stopped" }]))

    await user.click(screen.getByRole("button", { name: "Submit for review" }))
    expect(submitMutate).not.toHaveBeenCalled()
    expect(screen.getByText("End the open session?")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Submit and end the session" }))
    expect(submitMutate).toHaveBeenCalledTimes(1)
  })

  it("submits directly when no writer session is open", async () => {
    const user = userEvent.setup()
    renderHeader(chapter([]))

    await user.click(screen.getByRole("button", { name: "Submit for review" }))
    expect(submitMutate).toHaveBeenCalledTimes(1)
    expect(
      screen.queryByText("End the open session?")
    ).not.toBeInTheDocument()
  })

  it("hides Submit while the writer is running: the endpoint would refuse it", () => {
    renderHeader(chapter([{ role: "writer", status: "running" }]))
    expect(screen.queryByRole("button", { name: "Submit for review" })).not.toBeInTheDocument()
    expect(submitMutate).not.toHaveBeenCalled()
  })
})
