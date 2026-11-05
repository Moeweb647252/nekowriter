/** B-1: the Book agent entry must never fall through to a writer/other session. */
import { act, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { SessionDto } from "@/api/gen/SessionDto"

const sessions = vi.hoisted(() => ({ list: [] as SessionDto[] }))
vi.mock("@/api/hooks/chapters", () => ({ useChapter: () => ({ data: undefined, isPending: false }) }))
vi.mock("@/api/hooks/sessions", () => ({
  useBookSessions: () => ({ data: sessions.list, isPending: false }),
  useSession: (id: number | null) => ({
    data: id === null ? undefined : { session: sessions.list.find((s) => s.id === id), messages: [], tool_calls: [], next_cursor: null },
    isPending: false,
  }),
  useResumeSession: () => ({ mutate: vi.fn(), isPending: false }),
  useSendSessionMessage: () => ({ mutate: vi.fn(), isPending: false }),
  useSessionControl: () => ({ mutate: vi.fn(), isPending: false }),
  usePatchSession: () => ({ mutate: vi.fn(), isPending: false }),
  useStartDebugSession: () => ({ mutate: vi.fn(), isPending: false }),
  useEndSession: () => ({ mutate: vi.fn(), isPending: false }),
  useStartBookAgent: () => ({ mutate: vi.fn(), isPending: false }),
  useOlderMessages: () => ({ data: undefined, hasNextPage: false, isFetchingNextPage: false, isFetchNextPageError: false, error: null, fetchNextPage: vi.fn() }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn(), useWarningToasts: () => vi.fn() }))
vi.mock("sonner", () => ({ toast: { message: vi.fn(), success: vi.fn(), error: vi.fn(), warning: vi.fn() } }))

import { AgentPanel } from "@/features/agent/AgentPanel"
import { requestBookAgent } from "@/features/agent/bookAgentIntent"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useUiStore } from "@/stores/ui"

function writer(): SessionDto {
  return {
    id: 4, book_id: 3, role: "writer", kind: "chapter_write", chapter_id: 2, track_id: 1,
    research_task_id: null, chapter_number: 1, research_topic: null, research_task_kind: null,
    model_label: "Demo", status: "running", phase: "drafting", granularity: "paragraph",
    context_window: 100000,
    auto_pause: false, pause_after_outline: false, pause_requested: false,
    pause_after_step: false, held_tool_call_id: null,
    base_content_version: 0, error: null, usage: {}, counters: {}, warnings: [],
    created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z", finished_at: null,
  } as unknown as SessionDto
}

function planner(): SessionDto {
  return { ...writer(), id: 2, role: "planner", kind: "chapter_outline", status: "running" } as SessionDto
}

beforeEach(() => { sessions.list = [writer()] })

describe("Book agent open intent", () => {
  it("shows BookAgentStart instead of an open writer", () => {
    render(<TooltipProvider><AgentPanel bookId={3} /></TooltipProvider>)
    act(() => requestBookAgent(3))
    expect(screen.getByTestId("book-agent-start")).toBeInTheDocument()
    expect(screen.queryByText("Writing")).not.toBeInTheDocument()
  })

  it("keeps the planner reachable from the start form", async () => {
    const user = userEvent.setup()
    sessions.list = [planner()]
    render(<TooltipProvider><AgentPanel bookId={3} /></TooltipProvider>)
    act(() => requestBookAgent(3))
    expect(screen.getByTestId("book-agent-start")).toBeInTheDocument()
    expect(useUiStore.getState().pinnedSessionManual[3]).toBe(false)
    expect(screen.getByText("Outlining · Chapter 1")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Switch session" }))
    await user.click(screen.getByRole("menuitem", { name: /Outlining/ }))
    expect(screen.queryByTestId("book-agent-start")).not.toBeInTheDocument()
    expect(screen.getAllByText("Outlining · Chapter 1").length).toBeGreaterThan(0)
    expect(useUiStore.getState().pinnedSession[3]).toBe(2)
    expect(useUiStore.getState().pinnedSessionManual[3]).toBe(true)
  })
})
