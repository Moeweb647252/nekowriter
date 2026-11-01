/** Preparation tasks are knowledge checks in the research surfaces (D-158). */
import { render, screen } from "@testing-library/react"
import { MemoryRouter } from "react-router"
import { describe, expect, it, vi } from "vitest"

import type { ResearchTaskDto } from "@/api/gen/ResearchTaskDto"
import { ResearchTaskList } from "./ResearchTaskList"

function task(overrides: Partial<ResearchTaskDto> = {}): ResearchTaskDto {
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
    entries_created: 2,
    entries: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  }
}

const tasks = vi.hoisted(() => ({ rows: [] as ResearchTaskDto[] }))

vi.mock("@/api/hooks/research", () => ({
  useResearchTasks: () => ({ data: tasks.rows, isPending: false }),
  useDeferResearchTask: () => ({ mutate: vi.fn(), isPending: false }),
  useResumeResearchTask: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/features/research/useBulkDefer", () => ({
  useDeferReadyResearchTasks: () => ({ readyTasks: [], isPending: false, run: vi.fn() }),
}))

describe("ResearchTaskList preparation rows", () => {
  it("shows Knowledge check and an em dash target instead of the raw topic", () => {
    tasks.rows = [task()]
    render(
      <MemoryRouter>
        <ResearchTaskList />
      </MemoryRouter>,
    )
    expect(screen.getByText("Knowledge check")).toBeInTheDocument()
    expect(screen.queryByText("ignored preparation topic")).not.toBeInTheDocument()
    expect(screen.getByText(/—/)).toBeInTheDocument()
  })
})
