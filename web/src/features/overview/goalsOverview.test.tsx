/** The overview's ready-goals row and secondary next-action line (spec/08 §7, plan 13.9). */
import { render, screen } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"
import { MemoryRouter } from "react-router"

import type { NextActionDto } from "@/api/gen/NextActionDto"
import type { AttentionItemDto } from "@/api/gen/AttentionItemDto"
import type { TrackProgressDto } from "@/api/gen/TrackProgressDto"
import "@/i18n"

vi.mock("@/api/hooks/conception", () => ({
  useSeedCanon: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/api/hooks/reviews", () => ({
  useMaintainChapter: () => ({ mutate: vi.fn(), isPending: false }),
  useResyncChapter: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/api/hooks/tracks", () => ({
  useCreateChapter: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => vi.fn(),
  useWarningToasts: () => vi.fn(),
}))
vi.mock("@/features/workspace/shell", () => ({ useBookShell: () => null }))

import { AttentionList } from "@/features/overview/AttentionList"
import { NextActionCard } from "@/features/overview/NextActionCard"

const track: TrackProgressDto = {
  track_id: 10,
  kind: "main",
  title: "",
  chapters: 1,
  words: 100,
  by_status: [],
}

describe("overview ready goals", () => {
  it("links the attention row to the Goals Ready filter", () => {
    const items: AttentionItemDto[] = [
      { kind: "ready_goals", count: 2 },
    ]
    render(<MemoryRouter><AttentionList bookId={1} items={items} tracks={[track]} /></MemoryRouter>)
    expect(screen.getByText("2 ready goals")).toBeInTheDocument()
    expect(screen.getByRole("link", { name: /2 ready goals/ })).toHaveAttribute(
      "href",
      "/books/1/canon/goals?filter=ready",
    )
  })

  it("renders the secondary line under write_next only", () => {
    const action: NextActionDto = {
      kind: "write_next",
      chapter: null,
      track_id: 10,
      ready_research: null,
      ready_goals: 1,
      research_task_id: null,
    }
    render(<MemoryRouter><NextActionCard bookId={1} status="active" action={action} tracks={[track]} /></MemoryRouter>)
    expect(screen.getByRole("link", { name: "1 ready goal" })).toHaveAttribute(
      "href",
      "/books/1/canon/goals?filter=ready",
    )
  })
})
