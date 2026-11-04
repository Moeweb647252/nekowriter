/** The Goals page tree, conditions and readiness (spec/11 §5.5, spec/06 §11). */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, describe, expect, it, vi } from "vitest"

import enCanon from "@/i18n/locales/en/canon.json"
import { ApiError } from "@/api/client"
import type { GoalNodeDto } from "@/api/gen/GoalNodeDto"

const patchMutate = vi.hoisted(() => vi.fn())
const deleteMutate = vi.hoisted(() => vi.fn())

vi.mock("react-router", () => ({
  useParams: () => ({ bookId: "1" }),
  useSearchParams: () => [new URLSearchParams(), vi.fn()],
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => <a href={to}>{children}</a>,
}))

const nodes: GoalNodeDto[] = [
  {
    id: 1,
    book_id: 1,
    track_id: null,
    level: "goal",
    parent_id: null,
    title: "The heroine grows",
    purpose: "Make the reader believe she can choose.",
    status: "active",
    chapter_from: 1,
    chapter_to: 3,
    conditions: [],
    entities: [{ id: 7, name: "Lin Fan" }],
    events: [],
    foreshadowing: [],
    ready: false,
    closable: true,
    created_at: "",
    updated_at: "",
  },
  {
    id: 2,
    book_id: 1,
    track_id: null,
    level: "plan",
    parent_id: 1,
    title: "Choose the sword",
    purpose: "The choice must cost her the old life.",
    status: "waiting",
    chapter_from: null,
    chapter_to: null,
    conditions: [
      {
        condition: { kind: "chapter_at_least", chapter: 5 },
        satisfied: false,
        broken: false,
        target_label: null,
      },
      {
        condition: { kind: "event_occurred", event_id: 12 },
        satisfied: false,
        broken: false,
        target_label: "The gate",
      },
    ],
    entities: [],
    events: [{ id: 12, title: "The gate", status: "planned" }],
    foreshadowing: [],
    ready: true,
    closable: false,
    created_at: "",
    updated_at: "",
  },
  {
    id: 3,
    book_id: 1,
    track_id: null,
    level: "task",
    parent_id: 2,
    title: "Take it",
    purpose: "The scene where she takes it.",
    status: "done",
    chapter_from: null,
    chapter_to: null,
    conditions: [],
    entities: [],
    events: [],
    foreshadowing: [],
    ready: false,
    closable: false,
    created_at: "",
    updated_at: "",
  },
  {
    id: 4,
    book_id: 1,
    track_id: null,
    level: "task",
    parent_id: null,
    title: "Old idea",
    purpose: "Dropped.",
    status: "dropped",
    chapter_from: null,
    chapter_to: null,
    conditions: [],
    entities: [],
    events: [],
    foreshadowing: [],
    ready: false,
    closable: false,
    created_at: "",
    updated_at: "",
  },
]

vi.mock("@/api/hooks/goals", () => ({
  useGoals: () => ({ data: nodes, isPending: false, isError: false }),
  useCreateGoal: () => ({ mutate: vi.fn(), isPending: false }),
  usePatchGoal: () => ({ mutate: patchMutate, isPending: false }),
  useDeleteGoal: () => ({ mutate: deleteMutate, isPending: false }),
}))

vi.mock("@/api/hooks/tracks", () => ({
  useTracks: () => ({
    data: [
      { id: 1, kind: "main", title: "", canon_mode: "canon" },
      { id: 5, kind: "side", title: "Side", canon_mode: "isolated" },
    ],
  }),
  useChapters: () => ({ data: [] }),
}))

vi.mock("@/api/hooks/reviews", () => ({
  useProposals: () => ({ data: [], isPending: false }),
  useAcceptProposal: () => ({ mutate: vi.fn(), isPending: false }),
  useRejectProposal: () => ({ mutate: vi.fn(), isPending: false }),
}))

import GoalsPage from "@/features/canon/GoalsPage"

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

describe("GoalsPage", () => {
  it("renders purposes, statuses, ranges, linked chips and condition sentences", () => {
    render(<GoalsPage />)
    expect(screen.getByRole("heading", { name: "Goals" })).toBeInTheDocument()
    expect(screen.getByText("Make the reader believe she can choose.")).toBeInTheDocument()
    expect(screen.getByText("The choice must cost her the old life.")).toBeInTheDocument()
    expect(screen.getByText("Waiting for chapter 5")).toBeInTheDocument()
    expect(screen.getByText("Waiting for «The gate» to happen")).toBeInTheDocument()
    expect(screen.getByText("The gate")).toBeInTheDocument()
    expect(screen.getByText("Planned")).toBeInTheDocument()
    expect(screen.getByText("Chapters 1–3")).toBeInTheDocument()
    expect(screen.getByText("Lin Fan")).toBeInTheDocument()
    expect(screen.getAllByText("Waiting").length).toBeGreaterThan(0)
    expect(screen.getAllByText("Done").length).toBeGreaterThan(0)
  })

  it("shows the ready badge and closable prompt with Mark done", async () => {
    const user = userEvent.setup()
    render(<GoalsPage />)
    expect(screen.getAllByText("Ready").length).toBeGreaterThan(0)
    expect(screen.getByText("All children are closed")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Mark done" }))
    expect(patchMutate).toHaveBeenCalledWith(
      { id: 1, patch: { status: "done" } },
      expect.anything(),
    )
  })

  it("puts dropped nodes in a collapsed section", () => {
    render(<GoalsPage />)
    expect(screen.getByText("Dropped (1)")).toBeInTheDocument()
  })

  it("shows the Ready filter and its path", async () => {
    const user = userEvent.setup()
    render(<GoalsPage />)
    await user.click(screen.getByRole("button", { name: /Ready/ }))
    expect(screen.getByText("The heroine grows › Choose the sword")).toBeInTheDocument()
    expect(screen.queryByText("Take it")).toBeNull()
  })
})


describe("row-menu open children", () => {
  it("opens the confirmation and resends Mark done with confirm_open_children", async () => {
    patchMutate.mockImplementationOnce((_variables: unknown, options: { onError?: (error: unknown) => void }) => {
      options.onError?.(
        new ApiError(409, "goal.open_children", "open children", {
          children: [{ id: 12, level: "task", title: "Open child", status: "waiting" }],
        }),
      )
    })
    patchMutate.mockImplementationOnce(() => undefined)
    const user = userEvent.setup()
    render(<GoalsPage />)
    await user.click(screen.getByRole("button", { name: "Mark done" }))
    expect(screen.getByText("Open child")).toBeInTheDocument()
    expect(screen.getByText("Close this node anyway?")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Close anyway" }))
    expect(patchMutate.mock.calls[1][0]).toEqual({
      id: 1,
      patch: { status: "done", confirm_open_children: true },
    })
  })
})
