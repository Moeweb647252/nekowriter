/** Book › Knowledge: visible = checked, save sends unchecked, and the knowledge check button. */
import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { ResearchTaskDto } from "@/api/gen/ResearchTaskDto"

const state = vi.hoisted(() => ({
  scopes: [] as { id: number; name: string; description: string; entry_count: number }[],
  visible: [] as { id: number; name: string; description: string; entry_count: number }[],
  tasks: [] as ResearchTaskDto[],
}))

const save = vi.fn()
const startCheck = vi.fn()
const navigate = vi.fn()

vi.mock("@/api/hooks/knowledge", () => ({
  useScopes: () => ({ data: state.scopes, isPending: false }),
  useBookScopes: () => ({ data: state.visible, isPending: false }),
  usePutBookScopes: () => ({ mutate: save, isPending: false }),
}))
vi.mock("@/api/hooks/research", () => ({
  useResearchTasks: () => ({ data: state.tasks, isPending: false }),
  useStartKnowledgeCheck: () => ({ mutate: startCheck, isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))
vi.mock("react-router", () => ({
  useParams: () => ({ bookId: "1" }),
  useNavigate: () => navigate,
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => (
    <a href={to}>{children}</a>
  ),
}))
vi.mock("@/features/research/ResearchTaskList", () => ({ ResearchTaskList: () => null }))
vi.mock("@/features/research/NewResearchTaskDialog", () => ({ NewResearchTaskDialog: () => null }))

import BookKnowledgePage from "./BookKnowledgePage"

function scope(id: number, name = `Scope ${id}`) {
  return { id, name, description: "", entry_count: 0 }
}

function prep(overrides: Partial<ResearchTaskDto> = {}): ResearchTaskDto {
  return {
    id: 9,
    book_id: 1,
    scope_id: null,
    scope_name: "",
    topic: "Knowledge check",
    kind: "preparation",
    origin: "author",
    requirements: "",
    language: "en",
    status: "running",
    session_id: 3,
    report: "",
    entries_created: 0,
    entries: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  }
}

beforeEach(() => {
  state.scopes = [scope(1), scope(2)]
  state.visible = [scope(1)]
  state.tasks = []
  save.mockReset()
  startCheck.mockReset()
  navigate.mockReset()
})

describe("BookKnowledgePage", () => {
  it("starts with visible scopes checked and saves only the unchecked ids", async () => {
    const user = userEvent.setup()
    render(<BookKnowledgePage />)
    const checks = screen.getAllByRole("checkbox")
    expect(checks[0]).toBeChecked()
    expect(checks[1]).not.toBeChecked()
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(save).toHaveBeenCalledWith([2], expect.anything())
  })

  it("does not send a scope that appeared after the page loaded", async () => {
    const view = render(<BookKnowledgePage />)
    state.scopes = [scope(1), scope(2), scope(3)]
    view.rerender(<BookKnowledgePage />)
    await waitFor(() => expect(screen.getAllByRole("checkbox")).toHaveLength(3))
    await userEvent.click(screen.getByRole("button", { name: "Save" }))
    expect(save.mock.calls[0][0]).toEqual([2])
  })

  it("never sends a book-known scope that is absent from the loaded list", async () => {
    // The visible list includes a scope the page did not load into `useScopes` (for example a
    // second tab created it); the stale save must leave it alone rather than write an exclusion.
    state.scopes = [scope(1), scope(2)]
    state.visible = [scope(1), scope(3)]
    render(<BookKnowledgePage />)
    await userEvent.click(screen.getByRole("button", { name: "Save" }))
    expect(save.mock.calls[0][0]).toEqual([2])
  })

  it("disables Check knowledge while a check is open", () => {
    state.tasks = [prep()]
    render(<BookKnowledgePage />)
    const button = screen.getByRole("button", { name: "Check knowledge" })
    expect(button).toBeDisabled()
    expect(screen.getByText("A knowledge check is already open.")).toBeInTheDocument()
  })

  it("opens the focus dialog and navigates to the started task", async () => {
    startCheck.mockImplementation((_focus: string, options: { onSuccess: (value: unknown) => void }) => {
      options.onSuccess({ task: { id: 12 } })
    })
    const user = userEvent.setup()
    render(<BookKnowledgePage />)
    await user.click(screen.getByRole("button", { name: "Check knowledge" }))
    await user.type(screen.getByLabelText("Focus (optional)"), "Song tea houses")
    await user.click(screen.getByRole("button", { name: "Start check" }))
    expect(startCheck).toHaveBeenCalledWith("Song tea houses", expect.anything())
    expect(navigate).toHaveBeenCalledWith("/research/12")
  })
})
