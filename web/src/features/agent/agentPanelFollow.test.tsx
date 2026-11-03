/** The panel must follow a new open session off a terminal pin (author report, task-296). */
import "@/i18n"
import { act, render, screen } from "@testing-library/react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { SessionDto } from "@/api/gen/SessionDto"
import { useUiStore } from "@/stores/ui"

/** The session list the mocked hooks serve; a test mutates it to model a refetch. */
const server = vi.hoisted(() => ({ sessions: [] as SessionDto[] }))

/** A minimal research session; the reported case is a knowledge check on a book page. */
function researchSession(overrides: Partial<SessionDto>): SessionDto {
  return {
    id: 1,
    book_id: 3,
    role: "researcher",
    kind: "research",
    chapter_id: null,
    track_id: null,
    research_task_id: 1,
    research_task_kind: "preparation",
    chapter_number: null,
    research_topic: null,
    model_label: "GPT 6.1 Terra",
    status: "running",
    phase: "none",
    granularity: null,
    auto_pause: false,
    pause_after_outline: false,
    pause_requested: false,
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
    ...overrides,
  } as unknown as SessionDto
}

vi.mock("@/api/hooks/chapters", () => ({
  useChapter: () => ({ data: undefined, isPending: false }),
}))
vi.mock("@/api/hooks/sessions", () => ({
  useBookSessions: () => ({ data: server.sessions, isPending: false }),
  useSession: (id: number | null) => ({
    data:
      id === null
        ? undefined
        : {
            session: server.sessions.find((entry) => entry.id === id),
            messages: [],
            tool_calls: [],
            next_cursor: null,
          },
    isPending: false,
  }),
  useResumeSession: () => ({ mutate: vi.fn(), isPending: false }),
  useSendSessionMessage: () => ({ mutate: vi.fn(), isPending: false }),
  useSessionControl: () => ({ mutate: vi.fn(), isPending: false }),
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
  server.sessions = [
    researchSession({ id: 1, status: "abandoned" }),
    researchSession({ id: 2, status: "running" }),
  ]
  useUiStore.setState({ pinnedSession: { 3: 1 } })
})

describe("AgentPanel follows a new session off a terminal pin", () => {
  it("shows the running session, re-pins it, and stays there across a refetch", () => {
    const { rerender } = render(
      <TooltipProvider>
        <AgentPanel bookId={3} />
      </TooltipProvider>,
    )
    expect(screen.getByText("Running")).toBeInTheDocument()
    expect(screen.queryByText("Abandoned")).not.toBeInTheDocument()
    expect(useUiStore.getState().pinnedSession[3]).toBe(2)

    // A refetch after the followed session ends must keep the panel on session 2 instead of
    // falling back to the abandoned pin it left behind.
    act(() => {
      server.sessions = [
        researchSession({ id: 1, status: "abandoned" }),
        researchSession({ id: 2, status: "completed" }),
      ]
      rerender(
        <TooltipProvider>
          <AgentPanel bookId={3} />
        </TooltipProvider>,
      )
    })
    expect(screen.getByText("Completed")).toBeInTheDocument()
    expect(screen.queryByText("Abandoned")).not.toBeInTheDocument()
  })
})
