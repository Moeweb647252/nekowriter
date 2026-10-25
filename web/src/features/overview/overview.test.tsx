/**
 * The overview's next action and attention list (PLAN 11.1, spec/08 §7).
 *
 * The labels are composed from data (D-054), so these tests pin the localized text, the targets,
 * and the empty attention state.
 */
import { render, screen } from "@testing-library/react"
import { MemoryRouter, Route, Routes } from "react-router"
import { describe, expect, it, vi } from "vitest"

const overview = vi.fn()
vi.mock("@/api/hooks/books", () => ({
  useOverview: () => ({ data: overview(), isPending: false, isError: false }),
}))

vi.mock("@/api/hooks/conception", () => ({
  useSeedCanon: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/api/hooks/reviews", () => ({
  useResyncChapter: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/api/hooks/tracks", () => ({
  useCreateChapter: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => vi.fn(),
}))

import OverviewPage from "@/features/overview/OverviewPage"

const BOOK = {
  id: 1,
  title: "青云劫",
  idea: "",
  genre: "xianxia",
  writing_language: "zh-CN",
  status: "active",
  settings: { target_words_min: 3000, target_words_max: 4000 },
  model_overrides: {},
  conception: {
    session_id: null,
    characters_revision: 0,
    characters_approved_revision: null,
    topics_revision: 0,
    topics_approved_revision: null,
    finalized_at: null,
  },
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
}

const USAGE = {
  requests: 12,
  input_tokens: 3400,
  cached_input_tokens: 0,
  cache_write_tokens: 0,
  output_tokens: 900,
  reasoning_tokens: 0,
  last_prompt_tokens: 3400,
  estimated: false,
}

const TRACKS = [
  { track_id: 10, kind: "main", title: "", chapters: 1, words: 8, by_status: [] },
]

function data(overrides: Record<string, unknown>) {
  return {
    book: BOOK,
    next_action: null,
    attention: [],
    progress: {
      total_chapters: 1,
      official_chapters: 0,
      total_words: 8,
      official_words: 0,
      tracks: TRACKS,
    },
    recent_changes: [],
    usage: USAGE,
    ...overrides,
  }
}

function renderOverview() {
  return render(
    <MemoryRouter initialEntries={["/books/1"]}>
      <Routes>
        <Route path="/books/:bookId" element={<OverviewPage />} />
      </Routes>
    </MemoryRouter>,
  )
}

describe("OverviewPage", () => {
  it("renders the next action as a localized link", () => {
    overview.mockReturnValue(
      data({
        next_action: {
          kind: "review_chapter",
          chapter: { id: 5, number: 1, title: "One", status: "in_review" },
          track_id: 10,
        },
      }),
    )
    renderOverview()
    expect(screen.getByText("Next step")).toBeInTheDocument()
    expect(screen.getByText("Review Chapter 1")).toBeInTheDocument()
    const link = screen.getByRole("link", { name: /Go/ })
    expect(link).toHaveAttribute("href", "/books/1/chapters/5")
  })

  it("labels a side-story chapter with its track", () => {
    overview.mockReturnValue(
      data({
        next_action: {
          kind: "continue_chapter",
          chapter: { id: 7, number: 2, title: "Two", status: "drafting" },
          track_id: 11,
        },
        progress: {
          ...data({}).progress,
          tracks: [
            ...TRACKS,
            { track_id: 11, kind: "side", title: "Side Story", chapters: 1, words: 4, by_status: [] },
          ],
        },
      }),
    )
    renderOverview()
    expect(screen.getByText("Continue Side Story · Chapter 2")).toBeInTheDocument()
  })

  it("lists what needs the author, with the open session's title and status", () => {
    overview.mockReturnValue(
      data({
        attention: [
          {
            kind: "open_session",
            count: 1,
            session_id: 3,
            session_kind: "chapter_write",
            session_status: "paused",
            chapter: { id: 5, number: 1, title: "One", status: "drafting" },
            track_id: 10,
          },
          {
            kind: "pending_proposals",
            count: 2,
          },
          {
            kind: "ready_research",
            count: 1,
          },
        ],
      }),
    )
    renderOverview()
    expect(
      screen.getByText("Writing · Chapter 1 is Paused"),
    ).toBeInTheDocument()
    expect(screen.getByText("2 pending proposals")).toBeInTheDocument()
    expect(screen.getByText("1 research task is ready to run")).toBeInTheDocument()
    expect(screen.getByRole("link", { name: /Writing/ })).toHaveAttribute(
      "href",
      "/books/1/chapters/5",
    )
  })

  it("says that nothing needs the author when the list is empty", () => {
    overview.mockReturnValue(data({}))
    renderOverview()
    expect(screen.getByText("Nothing needs you right now.")).toBeInTheDocument()
  })

  it("shows the token usage totals", () => {
    overview.mockReturnValue(data({}))
    renderOverview()
    expect(
      screen.getByText("12 requests · 3,400 input tokens · 900 output tokens"),
    ).toBeInTheDocument()
  })
})
