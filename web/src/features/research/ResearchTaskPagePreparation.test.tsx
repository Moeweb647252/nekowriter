/** Preparation tasks have no topic/scope editor on the task page (D-158). */
import { render, screen } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import type { ResearchTaskDto } from "@/api/gen/ResearchTaskDto"

const state = vi.hoisted(() => ({ task: null as ResearchTaskDto | null }))

vi.mock("@/api/hooks/research", () => ({
  useResearchTask: () => ({ data: state.task, isPending: false }),
  usePatchResearchTask: () => ({ mutate: vi.fn(), isPending: false }),
  useDeleteResearchTask: () => ({ mutate: vi.fn(), isPending: false }),
  useRunResearchTask: () => ({ mutate: vi.fn(), isPending: false }),
  useDeferResearchTask: () => ({ mutate: vi.fn(), isPending: false }),
  useResumeResearchTask: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/api/hooks/sessions", () => ({
  useSession: () => ({ data: undefined }),
  useSessionControl: () => ({ mutate: vi.fn(), isPending: false }),
  useResumeSession: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))
vi.mock("react-router", () => ({
  useParams: () => ({ taskId: "9" }),
  useNavigate: () => vi.fn(),
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => (
    <a href={to}>{children}</a>
  ),
}))

import ResearchTaskPage from "./ResearchTaskPage"

function prep(): ResearchTaskDto {
  return {
    id: 9,
    book_id: 1,
    scope_id: null,
    scope_name: "",
    topic: "ignored preparation topic",
    kind: "preparation",
    origin: "author",
    requirements: "",
    language: "en",
    status: "done",
    session_id: null,
    report: "Report",
    entries_created: 0,
    entries: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  }
}

describe("ResearchTaskPage preparation", () => {
  it("shows Knowledge check with an em dash target and no topic editor", () => {
    state.task = prep()
    render(<ResearchTaskPage />)
    expect(screen.getByText("Knowledge check")).toBeInTheDocument()
    expect(screen.queryByText("ignored preparation topic")).not.toBeInTheDocument()
    expect(screen.getByText(/—/)).toBeInTheDocument()
    expect(screen.queryByLabelText("Topic")).not.toBeInTheDocument()
    expect(screen.queryByLabelText("Requirements")).not.toBeInTheDocument()
  })
})
