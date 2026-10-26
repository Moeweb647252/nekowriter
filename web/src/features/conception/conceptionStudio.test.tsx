/**
 * The conception studio (spec/08 §5, spec/11 §5.4, PROJECT §8.7):
 * - a conceiving book without an open conception session (created while no conception model was
 *   configured, or its session was abandoned) must offer **Start conception** — the endpoint
 *   exists and the studio is the only screen that can start it;
 * - the studio is where the author works with the conception agent, so the session controls
 *   (Pause/Stop while running, Continue when held) must be reachable from this route rather than
 *   only from the agent panel of the book workspace.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { SessionDto } from "@/api/gen/SessionDto"
import { TooltipProvider } from "@/components/ui/tooltip"

const startConception = vi.fn()
const answerSession = vi.fn()
const showError = vi.fn()
const sessionControl = vi.fn()
const resume = vi.fn()

const state = vi.hoisted(() => ({
  sessions: [] as unknown[],
  snapshot: undefined as unknown,
  docs: [] as unknown[],
  characters: [] as unknown[],
  topics: [] as unknown[],
}))

vi.mock("react-router", () => ({
  useParams: () => ({ bookId: "1" }),
  useNavigate: () => vi.fn(),
}))

const conceivingBook = {
  id: 1,
  title: "青云劫",
  status: "conceiving",
  conception: {
    session_id: null,
    characters_revision: 0,
    characters_approved_revision: null,
    topics_revision: 0,
    topics_approved_revision: null,
    finalized_at: null,
  },
  settings: { granularity: "paragraph" },
}

vi.mock("@/api/hooks/sessions", () => ({
  useBookSessions: () => ({ data: state.sessions, isPending: false }),
  useSession: () => ({ data: state.snapshot, isPending: false }),
  useAnswerSession: () => ({ mutate: answerSession, isPending: false }),
  useSendSessionMessage: () => ({ mutate: vi.fn(), isPending: false }),
  useSessionControl: () => ({ mutate: sessionControl, isPending: false }),
  useResumeSession: () => ({ mutate: resume, isPending: false }),
}))

vi.mock("@/api/hooks/settings-materials", () => ({
  useSettingDocs: () => ({ data: state.docs, isPending: false }),
  useCharacters: () => ({ data: state.characters, isPending: false }),
  usePutSettingDoc: () => ({ mutate: vi.fn(), isPending: false }),
  usePatchCharacter: () => ({ mutate: vi.fn(), isPending: false }),
  useDeleteCharacter: () => ({ mutate: vi.fn(), isPending: false }),
  useReorderCharacters: () => ({ mutate: vi.fn(), isPending: false }),
}))

vi.mock("@/api/hooks/research", () => ({
  useResearchTasks: () => ({ data: state.topics, isPending: false }),
  useCreateResearchTask: () => ({ mutate: vi.fn(), isPending: false }),
  usePatchResearchTask: () => ({ mutate: vi.fn(), isPending: false }),
  useDeleteResearchTask: () => ({ mutate: vi.fn(), isPending: false }),
}))

vi.mock("@/api/hooks/books", () => ({
  useBook: () => ({ data: conceivingBook, isPending: false, isError: false }),
}))

vi.mock("@/api/hooks/conception", () => ({
  useFinalizeConception: () => ({ mutate: vi.fn(), isPending: false }),
  useStartConception: () => ({ mutate: startConception, isPending: false }),
}))

vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => showError,
}))

import ConceptionStudio from "@/features/conception/ConceptionStudio"

/** The application wraps routed pages in the providers of `app/providers.tsx`. */
function renderStudio() {
  return render(
    <TooltipProvider>
      <ConceptionStudio />
    </TooltipProvider>,
  )
}

/** A minimal conception session in the given status. */
function conceptionSession(overrides: Partial<SessionDto>): SessionDto {
  return {
    id: 7,
    book_id: 1,
    role: "conception",
    kind: "conception",
    chapter_id: null,
    track_id: null,
    research_task_id: null,
    chapter_number: null,
    research_topic: null,
    model_label: "Demo model",
    status: "running",
    phase: "planning",
    granularity: null,
    auto_pause: false,
    pause_after_outline: false,
    pause_requested: false,
    held_tool_call_id: null,
    base_content_version: null,
    error: null,
    usage: {
      requests: 0,
      input_tokens: 0,
      cached_input_tokens: 0,
      cache_write_tokens: 0,
      output_tokens: 0,
      reasoning_tokens: 0,
      last_prompt_tokens: 0,
      estimated: false,
    },
    counters: {
      steps: 0,
      tool_calls: 0,
      retrieval_calls: 0,
      consecutive_tool_errors: 0,
      consecutive_nudges: 0,
      finish_warned: false,
    },
    parent_session_id: null,
    child_session_id: null,
    retrying: null,
    warnings: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    finished_at: null,
    ...overrides,
  }
}

describe("ConceptionStudio", () => {
  beforeEach(() => {
    state.sessions = []
    state.snapshot = undefined
    state.docs = []
    state.characters = []
    state.topics = []
    sessionControl.mockClear()
    resume.mockClear()
    answerSession.mockClear()
  })

  it("offers Start conception for a conceiving book", async () => {
    renderStudio()
    expect(screen.getByText(/No conception session yet/)).toBeInTheDocument()
    const button = screen.getByRole("button", { name: "Start conception" })
    await userEvent.click(button)
    expect(startConception).toHaveBeenCalledTimes(1)
  })

  it("offers Pause and Stop while the conception session runs", async () => {
    const session = conceptionSession({ status: "running" })
    state.sessions = [session]
    state.snapshot = { session, messages: [], system_prompt: "", tool_names: [] }
    renderStudio()
    await userEvent.click(screen.getByRole("button", { name: "Pause" }))
    expect(sessionControl).toHaveBeenCalledWith(
      { sessionId: 7, operation: "pause" },
      expect.objectContaining({ onError: showError }),
    )
    await userEvent.click(screen.getByRole("button", { name: "Stop" }))
    expect(sessionControl).toHaveBeenCalledWith(
      { sessionId: 7, operation: "stop" },
      expect.objectContaining({ onError: showError }),
    )
  })

  it("offers Continue for a held conception session and resumes it", async () => {
    const session = conceptionSession({ status: "held", held_tool_call_id: 3 })
    state.sessions = [session]
    state.snapshot = { session, messages: [], system_prompt: "", tool_names: [] }
    renderStudio()
    await userEvent.click(screen.getByRole("button", { name: "Continue" }))
    expect(resume).toHaveBeenCalledWith(
      { sessionId: 7 },
      expect.objectContaining({ onError: showError }),
    )
  })

  // PROJECT §8.7: the middle part needs its own scrollbar and must not share one with the
  // Outline / Characters / World / Protagonist / Research-topics column next to it.
  describe("scroll panes", () => {
    /** Long enough that both panes overflow in a real browser. */
    function longConversation() {
      const session = conceptionSession({ status: "running" })
      state.sessions = [session]
      state.snapshot = {
        session,
        messages: Array.from({ length: 40 }, (_, index) => ({
          message: {
            id: index + 1,
            seq: index,
            role: index % 2 === 0 ? "user" : "assistant",
            kind: index % 2 === 0 ? "kickoff" : "assistant",
            blocks: [{ type: "text", text: `message ${index}` }],
            stop_reason: null,
            usage: null,
            created_at: "2026-01-01T00:00:00Z",
          },
          tool_calls: [],
        })),
        system_prompt: "",
        tool_names: [],
      }
      return session
    }

    it("gives the conversation and the document column separate scroll containers", () => {
      longConversation()
      const { container } = renderStudio()

      const conversation = container.querySelector('[data-scroll-pane="conversation"]')
      const document = container.querySelector('[data-scroll-pane="document"]')
      expect(conversation).not.toBeNull()
      expect(document).not.toBeNull()
      expect(conversation).not.toBe(document)

      // Neither is an ancestor of the other, so scrolling one cannot move the other.
      expect(conversation?.contains(document as Node)).toBe(false)
      expect(document?.contains(conversation as Node)).toBe(false)
      // The tabs and their toolbar stay outside the document scroller: the tab row must not
      // scroll away with the text.
      const tabList = screen.getByRole("tablist")
      expect(document?.contains(tabList)).toBe(false)
    })

    it("keeps the composer and the finalize bar out of both scroll containers", () => {
      longConversation()
      const { container } = renderStudio()
      const conversation = container.querySelector('[data-scroll-pane="conversation"]')
      const document = container.querySelector('[data-scroll-pane="document"]')

      // The composer is pinned under the conversation, not inside its scroll area.
      const textarea = screen.getByRole("textbox", { name: "Message" })
      expect(conversation?.contains(textarea)).toBe(false)
      expect(document?.contains(textarea)).toBe(false)
      // The same for the finalize button: it is a footer of the document column.
      const finalize = screen.getByRole("button", { name: "Finalize book" })
      expect(conversation?.contains(finalize)).toBe(false)
      expect(document?.contains(finalize)).toBe(false)
    })

    it("bounds its own height, so the panes can scroll instead of the page", () => {
      longConversation()
      const { container } = renderStudio()
      const studio = container.querySelector('[data-testid="conception-studio"]')
      // `h-full` resolves against the definite height `BookLayout` gives this route only;
      // `overflow-hidden` is what keeps the composer inside the viewport.
      expect(studio?.className).toContain("h-full")
      expect(studio?.className).toContain("min-h-0")
      expect(studio?.className).toContain("overflow-hidden")
      // The scroll panes are `overflow-y-auto`, i.e. they are the ones with a scrollbar.
      for (const pane of container.querySelectorAll("[data-scroll-pane]")) {
        expect(pane.className).toContain("overflow-y-auto")
      }
    })
  })

  // task-41: the author's report. A `request_review` waiting for an answer must not open with every
  // verdict already chosen — 「通过」 then does nothing on a fresh row, which is what "点了没用" was.
  describe("request_review form", () => {
    /** The author's screen: a request_review call waiting, with all five items present. */
    function awaitingReview() {
      const session = conceptionSession({ status: "awaiting_input" })
      state.sessions = [session]
      state.snapshot = {
        session,
        messages: [
          {
            message: {
              id: 1,
              seq: 0,
              role: "assistant",
              kind: "assistant",
              blocks: [{ type: "text", text: "请审阅" }],
              stop_reason: null,
              usage: null,
              created_at: "2026-01-01T00:00:00Z",
            },
            tool_calls: [
              {
                id: 3,
                message_id: 1,
                seq: 0,
                call_index: 0,
                name: "request_review",
                input: { message: "请重点看冲突" },
                input_text: "{}",
                status: "awaiting_input",
                is_error: false,
                result_text: null,
                appendices: [],
                ui: { message: "请重点看冲突" },
                sealed: false,
                created_at: "2026-01-01T00:00:00Z",
                finished_at: null,
              },
            ],
          },
        ],
        system_prompt: "",
        tool_names: [],
      }
      return session
    }

    /** The five document/topic rows the author's screenshot showed. */
    function fiveItems() {
      state.docs = [
        { kind: "outline", content: "# 大纲", revision: 1, approved_revision: null },
        { kind: "world", content: "# 世界", revision: 1, approved_revision: null },
        { kind: "protagonist", content: "# 主角", revision: 1, approved_revision: null },
      ]
      state.characters = [{ id: 1, name: "青云" }]
      state.topics = [{ id: 1, status: "draft", topic: "唐代服饰" }]
      return ["outline", "characters", "world", "protagonist", "research_topics"] as const
    }

    it("shows all five rows unset, and only the clicked one becomes approved", async () => {
      const items = fiveItems()
      awaitingReview()
      renderStudio()

      for (const item of items) {
        const row = document.querySelector(`[data-review-item="${item}"]`)
        expect(row).not.toBeNull()
        expect((row as HTMLElement).dataset.verdict).toBe("unset")
      }

      // The reported click: 「通过」 on the outline row.
      const outlineRow = document.querySelector('[data-review-item="outline"]') as HTMLElement
      const approve = Array.from(outlineRow.querySelectorAll("button")).find((button) =>
        button.textContent?.includes("Approve"),
      ) as HTMLButtonElement
      expect(approve.getAttribute("aria-pressed")).toBe("false")
      await userEvent.click(approve)

      expect(outlineRow.dataset.verdict).toBe("approved")
      expect(approve.getAttribute("aria-pressed")).toBe("true")
      expect(approve.dataset.variant).toBe("default")
      // The other four rows are untouched — the form did not answer for the author.
      for (const item of items.filter((entry) => entry !== "outline")) {
        expect(
          (document.querySelector(`[data-review-item="${item}"]`) as HTMLElement).dataset.verdict,
        ).toBe("unset")
      }
    })

    it("sends the final verdicts through POST answer", async () => {
      const session = awaitingReview()
      fiveItems()
      renderStudio()

      const outlineRow = document.querySelector('[data-review-item="outline"]') as HTMLElement
      const outlineApprove = Array.from(outlineRow.querySelectorAll("button")).find((button) =>
        button.textContent?.includes("Approve"),
      ) as HTMLButtonElement
      await userEvent.click(outlineApprove)
      const topicsRow = document.querySelector(
        '[data-review-item="research_topics"]',
      ) as HTMLElement
      const topicsNeeds = Array.from(topicsRow.querySelectorAll("button")).find((button) =>
        button.textContent?.includes("Needs changes"),
      ) as HTMLButtonElement
      await userEvent.click(topicsNeeds)

      // The remaining three are answered with the explicit shortcut.
      await userEvent.click(screen.getByRole("button", { name: "Approve all" }))
      // Approve all covers everything, so re-state the rejection the author actually made.
      await userEvent.click(topicsNeeds)

      await userEvent.click(screen.getByRole("button", { name: "Submit review" }))
      expect(answerSession).toHaveBeenCalledTimes(1)
      const [variables] = answerSession.mock.calls[0]
      expect(variables).toEqual({
        sessionId: session.id,
        input: {
          tool_call_id: 3,
          payload: {
            docs: {
              outline: { approved: true },
              characters: { approved: true },
              world: { approved: true },
              protagonist: { approved: true },
              research_topics: { approved: false },
            },
          },
        },
      })
    })
  })
})
