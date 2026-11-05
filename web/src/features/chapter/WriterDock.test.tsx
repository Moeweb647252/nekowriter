import { fireEvent, render as rtlRender, screen, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { describe, expect, it, vi } from "vitest"

import type { SessionDto } from "@/api/gen/SessionDto"

const resume = vi.hoisted(() => vi.fn(async () => ({})))
const pause = vi.hoisted(() => vi.fn(async () => ({})))
const message = vi.hoisted(() => vi.fn(async () => ({})))
vi.mock("@/api/hooks/sessions", () => ({
  useResumeSession: () => ({ mutateAsync: resume, isPending: false }),
  useSessionControl: () => ({ mutateAsync: pause, isPending: false }),
  useSendSessionMessage: () => ({ mutateAsync: message, isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import { WriterDock } from "@/features/chapter/WriterDock"

function render(ui: React.ReactElement) {
  const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } })
  return rtlRender(<QueryClientProvider client={client}>{ui}</QueryClientProvider>)
}

function session(overrides: Partial<SessionDto> = {}): SessionDto {
  return {
    id: 4, book_id: 1, role: "writer", kind: "chapter_write", chapter_id: 1, track_id: 1,
    research_task_id: null, chapter_number: 1, research_topic: null, research_task_kind: null,
    model_label: "Demo", context_window: 1000, effort: null, status: "running", phase: "drafting",
    granularity: "paragraph", auto_pause: true, pause_after_outline: false, write_mode: null,
    pause_requested: false, pause_after_step: false, held_tool_call_id: null,
    base_content_version: 0, error: null, usage: {} as SessionDto["usage"],
    counters: {} as SessionDto["counters"], parent_session_id: null, child_session_id: null,
    retrying: null, warnings: [], created_at: "", updated_at: "", finished_at: null, ...overrides,
  }
}

describe("WriterDock", () => {
  it("is present while the writer runs and sends text as a steering message", async () => {
    render(<WriterDock bookId={1} chapterId={1} session={session()} held={undefined} view={undefined} />)
    const input = screen.getByRole("textbox", { name: "Message the writer" })
    fireEvent.change(input, { target: { value: "more" } })
    fireEvent.keyDown(input, { key: "Enter" })
    await waitFor(() => expect(message).toHaveBeenCalledWith({ sessionId: 4, text: "more" }))
  })

  it("focuses to request a step pause when auto-pause is off", async () => {
    render(<WriterDock bookId={1} chapterId={1} session={session({ auto_pause: false })} held={undefined} view={undefined} />)
    fireEvent.focus(screen.getByRole("textbox", { name: "Message the writer" }))
    await waitFor(() =>
      expect(pause).toHaveBeenCalledWith({
        sessionId: 4,
        operation: "pause",
        body: { after_step: true },
      }),
    )
  })

  it("is absent for a completed session", () => {
    const { container } = render(<WriterDock bookId={1} chapterId={1} session={session({ status: "completed" })} held={undefined} view={undefined} />)
    expect(container).toBeEmptyDOMElement()
  })
})
