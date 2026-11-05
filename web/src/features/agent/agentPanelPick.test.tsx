/** AgentPanel chooses the pinned chapter candidate, not the oldest open session (R3-08/T-02). */
import "@/i18n"
import { act, fireEvent, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import { useUiStore } from "@/stores/ui"

const planner = {
  id: 2,
  book_id: 3,
  role: "planner",
  kind: "chapter_outline",
  chapter_id: 2,
  track_id: 1,
  research_task_id: null,
  chapter_number: 1,
  research_topic: null,
  model_label: "Demo",
  context_window: 100000,
  status: "running",
  phase: "none",
  granularity: null,
  auto_pause: false,
  pause_after_outline: true,
  pause_requested: false,
    pause_after_step: false,
  held_tool_call_id: null,
  base_content_version: null,
  error: null,
  usage: {},
  counters: {},
  parent_session_id: null,
  child_session_id: null,
  retrying: null,
  warnings: [],
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  finished_at: null,
} as unknown as SessionDto
const reviewer = {
  ...planner,
  id: 1,
  role: "reviewer",
  kind: "chapter_review",
  status: "held",
}
const chapter = {
  id: 2,
  book_id: 3,
  track_id: 1,
  number: 1,
  title: "Chapter",
  status: "planned",
  canon_status: "none",
  word_count: 0,
  has_outline: false,
  content_version: 0,
  content: { paragraphs: [], next_pid: 1, tail_open: false },
  open_sessions: [reviewer, planner],
  guidance: "",
  outline: "",
  reference_notes: "",
  summary: "",
  official_version_id: null,
  updated_at: "2026-01-01T00:00:00Z",
} as unknown as ChapterDto

vi.mock("@/api/hooks/chapters", () => ({
  useChapter: () => ({ data: chapter, isPending: false }),
}))
const controlMutate = vi.hoisted(() => vi.fn())
vi.mock("@/api/hooks/sessions", () => ({
  useBookSessions: () => ({ data: [], isPending: false }),
  useSession: (id: number | null) => ({
    data: {
      session: id === 1 ? reviewer : planner,
      messages: [],
      tool_calls: [],
      next_cursor: null,
    },
    isPending: false,
  }),
  useResumeSession: () => ({ mutate: vi.fn(), isPending: false }),
  useSendSessionMessage: () => ({ mutate: vi.fn(), isPending: false }),
  useSessionControl: () => ({ mutate: controlMutate, isPending: false }),
  usePatchSession: () => ({ mutate: vi.fn(), isPending: false }),
  useStartDebugSession: () => ({ mutate: vi.fn(), isPending: false }),
  useOlderMessages: () => ({
    data: undefined,
    hasNextPage: false,
    isFetchingNextPage: false,
    isFetchNextPageError: false,
    error: null,
    fetchNextPage: vi.fn(),
  }),
}))
vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => vi.fn(),
  useWarningToasts: () => vi.fn(),
}))
vi.mock("sonner", () => ({ toast: { message: vi.fn(), success: vi.fn(), error: vi.fn(), warning: vi.fn() } }))

import { AgentPanel } from "@/features/agent/AgentPanel"
import { TooltipProvider } from "@/components/ui/tooltip"

beforeEach(() => {
  controlMutate.mockClear()
  useUiStore.setState({ pinnedSession: { 3: 2 } })
})

describe("AgentPanel pinned candidate", () => {
  it("shows the newly started planner over the older open reviewer", () => {
    render(
      <TooltipProvider>
        <AgentPanel bookId={3} chapterId={2} />
      </TooltipProvider>,
    )
    expect(screen.getByText("Outlining · Chapter 1")).toBeInTheDocument()
    expect(screen.queryByText("Reviewing · Chapter 1")).not.toBeInTheDocument()
  })

  it("does not re-pin a hotkey-controlled session after the author switched away", async () => {
    render(
      <TooltipProvider>
        <AgentPanel bookId={3} chapterId={2} />
      </TooltipProvider>,
    )
    act(() => {
      fireEvent.keyDown(window, { key: "p", altKey: true })
    })
    expect(controlMutate).toHaveBeenCalledTimes(1)
    const options = controlMutate.mock.calls[0]?.[1] as { onSuccess?: (updated: SessionDto) => void }
    useUiStore.setState({ pinnedSession: { 3: 3 } })
    act(() => {
      options.onSuccess?.({ ...planner, id: 2 } as SessionDto)
    })
    expect(useUiStore.getState().pinnedSession[3]).toBe(3)
  })

  it("does not re-pin a controlled session after the author switched away", async () => {
    const user = userEvent.setup()
    render(
      <TooltipProvider>
        <AgentPanel bookId={3} chapterId={2} />
      </TooltipProvider>,
    )
    await user.click(screen.getByRole("button", { name: "Pause" }))
    expect(controlMutate).toHaveBeenCalledTimes(1)
    const options = controlMutate.mock.calls[0]?.[1] as { onSuccess?: (updated: SessionDto) => void }
    useUiStore.setState({ pinnedSession: { 3: 3 } })
    act(() => {
      options.onSuccess?.({ ...planner, id: 2 } as SessionDto)
    })
    expect(useUiStore.getState().pinnedSession[3]).toBe(3)
  })
})
