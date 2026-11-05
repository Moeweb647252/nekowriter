import { fireEvent, render as rtlRender, screen, waitFor } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { beforeEach, describe, expect, it, vi } from "vitest"

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

function render(ui: React.ReactElement, client?: QueryClient) {
  const queryClient = client ?? new QueryClient({ defaultOptions: { mutations: { retry: false } } })
  return rtlRender(<QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>)
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
  beforeEach(() => {
    vi.clearAllMocks()
    vi.spyOn(document, "hasFocus").mockReturnValue(true)
  })

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

  it("blur with an empty input resumes after the dock's own pause", async () => {
    render(<WriterDock bookId={1} chapterId={1} session={session({ auto_pause: false })} held={undefined} view={undefined} />)
    const input = screen.getByRole("textbox", { name: "Message the writer" })
    fireEvent.focus(input)
    await waitFor(() => expect(pause).toHaveBeenCalledTimes(1))
    fireEvent.focusOut(input, { relatedTarget: document.body })
    await waitFor(() => expect(resume).toHaveBeenCalledWith({ sessionId: 4, keep_write: false }))
  })

  it("unsent text keeps the pause through a blur", async () => {
    render(<WriterDock bookId={1} chapterId={1} session={session({ auto_pause: false })} held={undefined} view={undefined} />)
    const input = screen.getByRole("textbox", { name: "Message the writer" })
    fireEvent.focus(input)
    await waitFor(() => expect(pause).toHaveBeenCalledTimes(1))
    fireEvent.change(input, { target: { value: "not sent yet" } })
    fireEvent.focusOut(input, { relatedTarget: document.body })
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(resume).not.toHaveBeenCalled()
  })

  it("keeps the pause when focus moves into the draft text", async () => {
    const { container } = render(
      <div data-draft-region>
        <p tabIndex={0}>draft paragraph</p>
        <WriterDock bookId={1} chapterId={1} session={session({ auto_pause: false })} held={undefined} view={undefined} />
      </div>,
    )
    const input = screen.getByRole("textbox", { name: "Message the writer" })
    const paragraph = container.querySelector("p") ?? document.body
    fireEvent.focus(input)
    await waitFor(() => expect(pause).toHaveBeenCalledTimes(1))
    fireEvent.focusOut(input, { relatedTarget: paragraph })
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(resume).not.toHaveBeenCalled()
  })

  it("ignores Shift+Enter and IME Enter", async () => {
    render(<WriterDock bookId={1} chapterId={1} session={session()} held={undefined} view={undefined} />)
    const input = screen.getByRole("textbox", { name: "Message the writer" })
    fireEvent.change(input, { target: { value: "more" } })
    fireEvent.keyDown(input, { key: "Enter", shiftKey: true })
    fireEvent.keyDown(input, { key: "Enter", keyCode: 229 })
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(message).not.toHaveBeenCalled()
  })

  it("serializes focus, blur and a new focus without a stale resume", async () => {
    let releasePause: (() => void) | undefined
    pause.mockImplementationOnce(
      () =>
        new Promise<object>((resolve) => {
          releasePause = () => resolve({})
        }),
    )
    render(<WriterDock bookId={1} chapterId={1} session={session({ auto_pause: false })} held={undefined} view={undefined} />)
    const input = screen.getByRole("textbox", { name: "Message the writer" })
    fireEvent.focus(input)
    fireEvent.focusOut(input, { relatedTarget: document.body })
    fireEvent.focus(input)
    releasePause?.()
    await new Promise((resolve) => setTimeout(resolve, 30))
    expect(resume).not.toHaveBeenCalled()
  })

  it("sends feedback once from a dock pause and does not queue a second resume after blur", async () => {
    render(<WriterDock bookId={1} chapterId={1} session={session({ auto_pause: false })} held={undefined} view={undefined} />)
    const input = screen.getByRole("textbox", { name: "Message the writer" })
    fireEvent.focus(input)
    await waitFor(() => expect(pause).toHaveBeenCalledTimes(1))
    fireEvent.change(input, { target: { value: "next note" } })
    fireEvent.keyDown(input, { key: "Enter" })
    await waitFor(() =>
      expect(resume).toHaveBeenCalledWith({ sessionId: 4, feedback: "next note", keep_write: false }),
    )
    await new Promise((resolve) => setTimeout(resolve, 30))
    expect(resume).toHaveBeenCalledTimes(1)
  })

  it("waits for an in-flight content save before the leave resume", async () => {
    const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } })
    let checks = 0
    vi.spyOn(client, "isMutating").mockImplementation(() => {
      checks += 1
      return checks < 3 ? 1 : 0
    })
    render(
      <WriterDock bookId={1} chapterId={1} session={session({ auto_pause: false })} held={undefined} view={undefined} />,
      client,
    )
    const input = screen.getByRole("textbox", { name: "Message the writer" })
    fireEvent.focus(input)
    await waitFor(() => expect(pause).toHaveBeenCalledTimes(1))
    fireEvent.focusOut(input, { relatedTarget: document.body })
    await new Promise((resolve) => setTimeout(resolve, 10))
    expect(resume).not.toHaveBeenCalled()
    await waitFor(() => expect(resume).toHaveBeenCalledWith({ sessionId: 4, keep_write: false }), {
      timeout: 1000,
    })
  })
