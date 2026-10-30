/** Goal proposal cards render their target and edits (spec/11 §5.5, plan 13.9). */
import { render, screen } from "@testing-library/react"
import i18n from "i18next"
import { beforeAll, describe, expect, it, vi } from "vitest"

import enBook from "@/i18n/locales/en/book.json"
import enCanon from "@/i18n/locales/en/canon.json"
import type { ProposalDto } from "@/api/gen/ProposalDto"

vi.mock("@/api/hooks/reviews", () => ({
  useAcceptProposal: () => ({ mutate: vi.fn(), isPending: false }),
  useRejectProposal: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import { ProposalCard } from "@/features/proposals/ProposalCard"

const base = {
  id: 1,
  book_id: 1,
  target_kind: null,
  target_id: null,
  rationale: "because",
  status: "pending" as const,
  chapter_id: null,
  session_id: null,
  created_at: "2026-01-01T00:00:00Z",
  decided_at: null,
  preview: { before: "", after: "" },
}

beforeAll(() => {
  i18n.addResourceBundle("en", "book", enBook, true, true)
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

describe("ProposalCard goal renderers", () => {
  it("renders a status proposal on its node", () => {
    const proposal: ProposalDto = {
      ...base,
      target_type: "goal_node",
      edits: { kind: "set_goal_status", status: "done" },
      goal_target: { id: 4, level: "task", title: "Take the sword", status: "active" },
    }
    render(<ProposalCard proposal={proposal} bookId={1} unspaced={false} />)
    expect(screen.getByText("Goal node")).toBeInTheDocument()
    expect(screen.getByText("Take the sword")).toBeInTheDocument()
    expect(screen.getByText("Take the sword → Done")).toBeInTheDocument()
  })

  it("renders a new task proposal with its purpose", () => {
    const proposal: ProposalDto = {
      ...base,
      target_type: "new_goal_task",
      edits: {
        kind: "create_goal_task",
        task: {
          parent_id: 3,
          title: "Find the sword",
          purpose: "The clue matters.",
          chapter_from: null,
          chapter_to: null,
          entity_ids: [],
          event_ids: [],
          foreshadowing_ids: [],
          track_id: null,
        },
      },
      goal_target: { id: 3, level: "plan", title: "The plan", status: "active" },
    }
    render(<ProposalCard proposal={proposal} bookId={1} unspaced={false} />)
    expect(screen.getByText("New goal task")).toBeInTheDocument()
    expect(screen.getAllByText("Find the sword").length).toBeGreaterThan(0)
    expect(screen.getByText("The clue matters.")).toBeInTheDocument()
    expect(screen.getByText("Under The plan")).toBeInTheDocument()
  })
})
