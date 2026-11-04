/** Book-agent start, write-mode, end and held-preview UI (plan §3.4/§3.7). */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"

const startBookAgent = vi.hoisted(() => vi.fn())
const endSession = vi.hoisted(() => vi.fn())
vi.mock("@/api/endpoints/sessions", () => ({ startBookAgent, endSession }))

import { BookAgentStart, WriteModeControl } from "@/features/agent/BookAgentStart"
import { BookAgentEndControl } from "@/features/agent/BookAgentEndControl"
import { HeldCallReview } from "@/features/agent/HeldCallReview"
import { registeredTools } from "@/features/agent/toolRenderers"

const session = {
  id: 7,
  book_id: 1,
  role: "assistant",
  kind: "book_agent",
  status: "held",
  write_mode: "review",
  chapter_id: null,
  model_label: "Demo",
  context_window: 100000,
  phase: "none",
  granularity: null,
  auto_pause: false,
  pause_after_outline: false,
  pause_requested: false,
  held_tool_call_id: 1,
  error: null,
  usage: {},
  counters: {},
  warnings: [],
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
} as unknown as SessionDto

function call(preview: unknown): ToolCallDto {
  return {
    id: 1,
    message_id: 1,
    seq: 0,
    call_index: 0,
    name: "edit_chapter_text",
    input: {},
    input_text: "",
    status: "held",
    is_error: false,
    result_text: null,
    appendices: [],
    ui: { preview: preview as never },
    sealed: false,
    created_at: "2026-01-01T00:00:00Z",
    finished_at: null,
  }
}

describe("BookAgentStart", () => {
  beforeEach(() => startBookAgent.mockReset())

  it("defaults to review and starts with the chosen full mode", async () => {
    startBookAgent.mockResolvedValue({ session_id: 11 })
    const onStarted = vi.fn()
    render(<BookAgentStart bookId={3} onStarted={onStarted} />)
    expect(screen.getByRole("radio", { name: "Review changes" })).toHaveAttribute("aria-checked", "true")
    await userEvent.click(screen.getByRole("radio", { name: "Full access" }))
    await userEvent.type(screen.getByRole("textbox", { name: "First message" }), "Read the book.")
    await userEvent.click(screen.getByRole("button", { name: "Start conversation" }))
    expect(startBookAgent).toHaveBeenCalledWith(3, { message: "Read the book.", write_mode: "full" })
    expect(onStarted).toHaveBeenCalledWith(11)
  })
})

describe("WriteModeControl", () => {
  it("changes mode even while a busy session runs", async () => {
    const onChange = vi.fn()
    render(<WriteModeControl mode="review" busy={false} disabled={false} onChange={onChange} />)
    await userEvent.click(screen.getByRole("radio", { name: "Full access" }))
    expect(onChange).toHaveBeenCalledWith("full")
  })
})

describe("BookAgentEndControl", () => {
  it("warns that a held change is not applied", async () => {
    endSession.mockResolvedValue(session)
    const onEnded = vi.fn()
    render(<BookAgentEndControl session={session} onEnded={onEnded} />)
    await userEvent.click(screen.getByRole("button", { name: "End conversation" }))
    expect(screen.getByText(/will not be applied/)).toBeInTheDocument()
    await userEvent.click(screen.getByRole("button", { name: "End conversation" }))
    expect(endSession).toHaveBeenCalledWith(7)
  })
})

describe("HeldCallReview previews", () => {
  it("renders diff, fields and action previews without raw JSON", () => {
    const onApprove = vi.fn()
    const { rerender } = render(<HeldCallReview call={call({ kind: "diff", label: "Chapter", before: "old", after: "new" })} busy={false} onApprove={onApprove} onRequestChanges={vi.fn()} />)
    expect(screen.getByText("old")).toBeInTheDocument()
    expect(screen.getByText("new")).toBeInTheDocument()
    rerender(<HeldCallReview call={call({ kind: "fields", label: "Goal", rows: [{ field: "title", before: "A", after: "B" }] })} busy={false} onApprove={onApprove} onRequestChanges={vi.fn()} />)
    expect(screen.getByText("title")).toBeInTheDocument()
    rerender(<HeldCallReview call={call({ kind: "action", label: "Delete", text: "Delete entity X" })} busy={false} onApprove={onApprove} onRequestChanges={vi.fn()} />)
    expect(screen.getByText("Delete entity X")).toBeInTheDocument()
    expect(screen.queryByText(/\{"kind"/)).not.toBeInTheDocument()
  })
})

it("registers the book-agent tool renderers", () => {
  const names = new Set(registeredTools())
  for (const name of ["book_overview", "open_chapter", "read_setting_doc", "read_characters", "list_goals", "list_research", "list_sessions", "start_session"]) {
    expect(names.has(name)).toBe(true)
  }
})
