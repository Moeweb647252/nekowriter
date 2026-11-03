/** The research-topics tab labels author-created rows (D-158). */
import { render, screen } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import type { ResearchTaskDto } from "@/api/gen/ResearchTaskDto"
import { ResearchTopicsEditor } from "./ResearchTopicsEditor"

function topic(overrides: Partial<ResearchTaskDto> = {}): ResearchTaskDto {
  return {
    id: 1,
    book_id: 1,
    scope_id: null,
    scope_name: "History",
    topic: "A topic",
    kind: "topic",
    origin: "author",
    requirements: "",
    language: "en",
    status: "draft",
    session_id: null,
    check_id: null,
    report: "",
    entries_created: 0,
    entries: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  }
}

describe("ResearchTopicsEditor", () => {
  it("labels only author-created rows", () => {
    render(
      <ResearchTopicsEditor
        topics={[topic({ id: 1, topic: "Mine", origin: "author" }), topic({ id: 2, topic: "Agent", origin: "conception" })]}
        editable={false}
        busy={false}
        onCreate={vi.fn()}
        onPatch={vi.fn()}
        onDelete={vi.fn()}
      />,
    )
    expect(screen.getByText("Added by you")).toBeInTheDocument()
    expect(screen.getAllByText("Added by you")).toHaveLength(1)
    expect(screen.getByDisplayValue("Mine")).toBeInTheDocument()
    expect(screen.getByDisplayValue("Agent")).toBeInTheDocument()
  })
})
