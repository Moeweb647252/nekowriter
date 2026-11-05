/** Chapter header actions per outline state (PROJECT §8.5, D-146). */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { TooltipProvider } from "@/components/ui/tooltip"
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { BookSettings } from "@/api/gen/BookSettings"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { SessionDto } from "@/api/gen/SessionDto"

vi.mock("@/api/hooks/chapters", () => ({
  useSubmitChapter: () => ({ mutate: vi.fn(), isPending: false }),
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
vi.mock("@/features/chapter/OutlineDialog", () => ({ OutlineDialog: () => null }))
vi.mock("@/features/chapter/SendBackDialog", () => ({ SendBackDialog: () => null }))
vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => vi.fn(),
  useWarningToasts: () => vi.fn(),
}))

import { ChapterHeader } from "@/features/chapter/ChapterHeader"

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
  canon_systems: { entities: true, timeline: true, memory: true, foreshadowing: true, goals: true },
}

function outlineSession(): SessionDto {
  return {
    id: 9,
    book_id: 1,
    role: "planner",
    kind: "chapter_outline",
    status: "running",
  } as unknown as SessionDto
}

function chapter(overrides: Partial<ChapterDto> = {}): ChapterDto {
  return {
    id: 4,
    book_id: 1,
    track_id: 2,
    number: 4,
    title: "Four",
    status: "planned",
    canon_status: "none",
    word_count: 0,
    has_outline: false,
    updated_at: "2026-01-01T00:00:00Z",
    guidance: "",
    outline: "",
    reference_notes: "",
    summary: "",
    content: { paragraphs: [], next_pid: 1, tail_open: false },
    content_version: 0,
    official_version_id: null,
    open_sessions: [],
    ...overrides,
  }
}

function renderHeader(value: ChapterDto, outlineState: "missing" | "accepted") {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
      <ChapterHeader
        chapter={value}
        settings={SETTINGS}
        language="en"
        outlineState={outlineState}
        onRename={() => {}}
      />
      </TooltipProvider>
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  vi.clearAllMocks()
})

describe("ChapterHeader outline actions", () => {
  it("offers Write the outline on a planned chapter with no outline", () => {
    renderHeader(chapter(), "missing")
    expect(screen.getByRole("button", { name: "Write the outline" })).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Write the chapter" })).not.toBeInTheDocument()
  })

  it("offers Write the chapter and Redo the outline on an accepted outline", () => {
    renderHeader(chapter({ outline: "细纲", has_outline: true }), "accepted")
    expect(screen.getByRole("button", { name: "Write the chapter" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Redo the outline" })).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Write the outline" })).not.toBeInTheDocument()
  })

  it("hides start actions while an outline session is open", () => {
    renderHeader(chapter({ open_sessions: [outlineSession()] }), "missing")
    expect(screen.queryByRole("button", { name: "Write the outline" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Write the chapter" })).not.toBeInTheDocument()
  })

  it("offers Redo the outline on an in-review chapter, but never on an official one", () => {
    renderHeader(
      chapter({ status: "in_review", outline: "细纲", has_outline: true }),
      "accepted",
    )
    expect(screen.getByRole("button", { name: "Redo the outline" })).toBeInTheDocument()
  })

  it("hides Approve while a planner session is open", () => {
    renderHeader(
      chapter({
        status: "in_review",
        outline: "细纲",
        has_outline: true,
        open_sessions: [outlineSession()],
      }),
      "accepted",
    )
    expect(screen.queryByRole("button", { name: "Approve" })).not.toBeInTheDocument()
  })

  it("disables Submit while the writer is running", async () => {
    const user = userEvent.setup()
    renderHeader(
      chapter({
        status: "drafting",
        outline: "细纲",
        has_outline: true,
        open_sessions: [
          { id: 8, book_id: 1, role: "writer", kind: "chapter_write", status: "running" } as never,
        ],
      }),
      "accepted",
    )
    const button = screen.getByRole("button", { name: "Submit for review" })
    expect(button).toBeDisabled()
    await user.hover(button)
    expect(await screen.findByText("The writer is running. Pause or stop it to submit.")).toBeInTheDocument()
  })

  it("keeps Submit enabled while a writer session is held", () => {
    renderHeader(
      chapter({
        status: "drafting",
        outline: "细纲",
        has_outline: true,
        content: {
          paragraphs: [{ pid: "p1", text: "一。" }],
          next_pid: 2,
          tail_open: false,
        },
        open_sessions: [
          { id: 8, book_id: 1, role: "writer", kind: "chapter_write", status: "held" } as never,
        ],
      }),
      "accepted",
    )
    expect(screen.getByRole("button", { name: "Submit for review" })).toBeEnabled()
  })

  it("does not offer Redo the outline on an official chapter", () => {
    renderHeader(
      chapter({ status: "official", outline: "细纲", has_outline: true }),
      "accepted",
    )
    expect(screen.queryByRole("button", { name: "Redo the outline" })).not.toBeInTheDocument()
  })
})
