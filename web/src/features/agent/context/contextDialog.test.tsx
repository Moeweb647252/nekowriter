import { fireEvent, render, screen } from "@testing-library/react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { ContextBreakdownDto } from "@/api/gen/ContextBreakdownDto"

const state = vi.hoisted(() => ({ view: null as ContextBreakdownDto | null }))
vi.mock("@/api/hooks/sessions", () => ({
  useSessionContext: () => ({ data: state.view, isPending: false, isError: false }),
}))

import { ContextDialog } from "./ContextDialog"
import { useUiStore } from "@/stores/ui"

function view(): ContextBreakdownDto {
  return {
    session_id: 7, seq: null, context_window: 100000, max_output_tokens: 4096, rollover_at: 80904,
    estimate: 1000, measured: null, cached: null, scale: 1, calibrated: false, pending_calls: 0,
    groups: [{
      key: "author", estimate: 1000, parts: [{ key: "message", name: null, estimate: 1000, items: [{ seq: 1, call_index: null, estimate: 1000, excerpt: "hello" }], rest: null }],
    }],
    requests: [{ seq: 1, prompt_tokens: 0, cached_tokens: 0, estimated: true, estimate: 1000, created_at: "2026-01-01T00:00:00Z" }],
  }
}

beforeEach(() => {
  state.view = view()
  useUiStore.getState().setContextView("treemap")
})

describe("ContextDialog", () => {
  it("renders the estimate marker, groups and legend", () => {
    render(<ContextDialog sessionId={7} open onOpenChange={vi.fn()} />)
    expect(screen.getByText("Estimate")).toBeInTheDocument()
    expect(screen.getByText("From the author")).toBeInTheDocument()
    expect(screen.getByText(/Setup: system prompt/)).toBeInTheDocument()
    expect(screen.getByRole("slider")).toBeInTheDocument()
  })

  it("switches to the list view and shows the same numbers", () => {
    render(<ContextDialog sessionId={7} open onOpenChange={vi.fn()} />)
    fireEvent.click(screen.getByRole("button", { name: "List" }))
    expect(screen.getByRole("columnheader", { name: "Name" })).toBeInTheDocument()
    expect(screen.getByText("hello")).toBeInTheDocument()
  })

  it("zooms into a group and back out with Escape", () => {
    render(<ContextDialog sessionId={7} open onOpenChange={vi.fn()} />)
    fireEvent.click(screen.getByTestId("context-cell-group-author"))
    expect(screen.getByText("› From the author")).toBeInTheDocument()
    const treemap = screen.getByTestId("context-treemap")
    fireEvent.keyDown(treemap, { key: "Escape" })
    expect(screen.queryByText("› From the author")).not.toBeInTheDocument()
  })
})

  it("uses the wide sm dialog width and keeps the treemap inside a min-w-0 wrapper", () => {
    render(<ContextDialog sessionId={7} open onOpenChange={vi.fn()} />)
    expect(screen.getByTestId("context-dialog")).toHaveClass("sm:max-w-5xl")
    expect(screen.getByTestId("context-scroll")).toHaveClass("min-w-0")
    expect(screen.getByTestId("context-treemap")).toHaveClass("min-w-0", "max-w-full")
    expect(screen.getByTestId("context-treemap").querySelector("svg")).toHaveClass("max-w-full")
  })

function subdividedView(): ContextBreakdownDto {
  const keys = [
    "block.settings.outline",
    "block.settings.characters",
    "block.settings.world",
    "block.settings.protagonist",
    "block.memory.recent",
    "block.memory.entities",
    "block.memory.facts",
    "block.memory.foreshadowing",
  ]
  return {
    session_id: 7, seq: null, context_window: 100000, max_output_tokens: 4096, rollover_at: null,
    estimate: 1000, measured: null, cached: null, scale: 1, calibrated: false, pending_calls: 0,
    groups: [
      {
        key: "kickoff", estimate: 800,
        parts: keys.map((name) => ({
          key: "block", name, estimate: 100,
          items: [{ seq: 1, call_index: null, estimate: 100, excerpt: name }], rest: null,
        })),
      },
      {
        key: "results", estimate: 200,
        parts: [
          { key: "result", name: "insert_text", estimate: 120, items: [], rest: null },
          { key: "result", name: "finish_draft", estimate: 80, items: [], rest: null },
        ],
      },
    ],
    requests: [],
  }
}

it("labels the eight kickoff subdivisions and keeps one part per tool result", () => {
  state.view = subdividedView()
  render(<ContextDialog sessionId={7} open onOpenChange={vi.fn()} />)
  fireEvent.click(screen.getByTestId("context-cell-group-kickoff"))
  for (const label of ["Outline", "Characters", "World", "Protagonist", "Recent", "Entities", "Facts", "Foreshadowing"]) {
    expect(screen.getByText(label)).toBeInTheDocument()
  }
  fireEvent.click(screen.getByRole("button", { name: "List" }))
  for (const label of ["Outline", "Characters", "World", "Protagonist", "Recent", "Entities", "Facts", "Foreshadowing"]) {
    expect(screen.getAllByText(label).length).toBeGreaterThan(0)
  }
  expect(screen.getByText("insert_text")).toBeInTheDocument()
  expect(screen.getByText("finish_draft")).toBeInTheDocument()
})
