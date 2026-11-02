/**
 * The conception studio (spec/08 §5, spec/11 §5.4, PROJECT §8.7):
 * - a conceiving book without an open conception session (created while no conception model was
 *   configured, or its session was abandoned) must offer **Start conception** — the endpoint
 *   exists and the studio is the only screen that can start it;
 * - the studio is where the author works with the conception agent, so the session controls
 *   (Pause/Stop while running, Continue when held) must be reachable from this route rather than
 *   only from the agent panel of the book workspace.
 */
import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { beforeEach, describe, expect, it, vi } from "vitest"

import { applyServerEvent } from "@/api/events"

import type { SessionDto } from "@/api/gen/SessionDto"
import { TooltipProvider } from "@/components/ui/tooltip"
import i18n from "i18next"
import zhAgent from "@/i18n/locales/zh-CN/agent.json"
import zhBook from "@/i18n/locales/zh-CN/book.json"
import zhChapter from "@/i18n/locales/zh-CN/chapter.json"

const startConception = vi.fn()
const startKnowledgeCheck = vi.fn()
const runResearchTask = vi.fn()
const answerSession = vi.fn()
const showError = vi.fn()
const warningToast = vi.fn()
const finalize = vi.fn()
const sessionControl = vi.fn()
const resume = vi.fn()

const state = vi.hoisted(() => ({
  sessions: [] as unknown[],
  snapshot: undefined as unknown,
  checkSnapshot: undefined as unknown,
  docs: [] as unknown[],
  characters: [] as unknown[],
  topics: [] as unknown[],
}))

vi.mock("react-router", () => ({
  useParams: () => ({ bookId: "1" }),
  useNavigate: () => vi.fn(),
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => (
    <a href={to}>{children}</a>
  ),
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

vi.mock("@/api/hooks/sessions", async () => {
  const { useQuery } = await import("@tanstack/react-query")
  const { keys } = await import("@/api/keys")
  return {
    useBookSessions: (bookId: number) =>
      useQuery({
        queryKey: keys.sessions.list(bookId),
        queryFn: async () => state.sessions,
        initialData: state.sessions,
      }),
    useSession: (sessionId: number | null) => ({
      data: sessionId === 11 ? state.checkSnapshot : state.snapshot,
      isPending: false,
    }),
    useAnswerSession: () => ({ mutate: answerSession, isPending: false }),
    useSendSessionMessage: () => ({ mutate: vi.fn(), isPending: false }),
    useSessionControl: () => ({ mutate: sessionControl, isPending: false }),
    useResumeSession: () => ({ mutate: resume, isPending: false }),
    useOlderMessages: () => ({
      data: undefined,
      hasNextPage: false,
      isFetchingNextPage: false,
      isFetchNextPageError: false,
      error: null,
      fetchNextPage: vi.fn(),
    }),
  }
})

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
  useStartKnowledgeCheck: () => ({ mutate: startKnowledgeCheck, isPending: false }),
  useRunResearchTask: () => ({ mutate: runResearchTask, isPending: false }),
}))

vi.mock("@/api/hooks/books", () => ({
  useBook: () => ({ data: conceivingBook, isPending: false, isError: false }),
}))

vi.mock("@/api/hooks/conception", () => ({
  useFinalizeConception: () => ({ mutate: finalize, isPending: false }),
  useStartConception: () => ({ mutate: startConception, isPending: false }),
}))

vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => showError,
  useWarningToasts: () => warningToast,
}))

// jsdom has no layout, so the real virtualizer reports no visible rows. Render every row so the
// check-open assertions can see the scripted message.
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getTotalSize: () => count * 120,
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({ index, start: index * 120 })),
    measureElement: () => undefined,
    scrollToEnd: () => undefined,
  }),
}))

import ConceptionStudio from "@/features/conception/ConceptionStudio"

/** The application wraps routed pages in the providers of `app/providers.tsx`. */
function makeClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } })
}

function renderStudio(client: QueryClient = makeClient()) {
  return render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <ConceptionStudio />
      </TooltipProvider>
    </QueryClientProvider>,
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
    state.checkSnapshot = undefined
    state.docs = []
    state.characters = []
    state.topics = []
    sessionControl.mockClear()
    warningToast.mockClear()
    finalize.mockClear()
    startKnowledgeCheck.mockClear()
    runResearchTask.mockClear()
    resume.mockClear()
    answerSession.mockClear()
  })

  it("offers Start conception for a conceiving book and surfaces its warnings", async () => {
    startConception.mockImplementation(
      (_input: unknown, options?: { onSuccess?: (response: unknown) => void }) => {
        options?.onSuccess?.({ warnings: ["model.effort_unsupported"] })
      },
    )
    renderStudio()
    expect(screen.getByText(/No conception session yet/)).toBeInTheDocument()
    const button = screen.getByRole("button", { name: "Start conception" })
    await userEvent.click(button)
    expect(startConception).toHaveBeenCalledTimes(1)
    expect(warningToast).toHaveBeenCalledWith(["model.effort_unsupported"])
  })

  it("surfaces warnings from a successful finalize seed", async () => {
    state.docs = [
      { kind: "outline", content: "# Outline", revision: 1, approved_revision: 1 },
      { kind: "world", content: "# World", revision: 1, approved_revision: 1 },
      { kind: "protagonist", content: "# Hero", revision: 1, approved_revision: 1 },
    ]
    state.characters = [{ id: 1 }]
    finalize.mockImplementation(
      (_input: unknown, options?: { onSuccess?: (response: unknown) => void }) => {
        options?.onSuccess?.({ book: { id: 1 }, warnings: ["model.effort_unsupported"] })
      },
    )
    renderStudio()
    await userEvent.click(screen.getByRole("button", { name: "Finalize book" }))
    // The fixture has unapproved revisions, so the confirmation gate appears first.
    const confirmation = screen.queryByRole("alertdialog")
    if (confirmation !== null) {
      await userEvent.click(
        screen.getByRole("button", { name: i18n.t("common:confirm") as string }),
      )
    }
    expect(warningToast).toHaveBeenCalledWith(["model.effort_unsupported"])
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


/** A preparation task shown by the studio before a conception session exists. */
function preparationTask(overrides: Partial<Record<string, unknown>> = {}) {
  return {
    id: 21,
    book_id: 1,
    scope_id: null,
    scope_name: "History",
    topic: "Knowledge check",
    kind: "preparation",
    origin: "author",
    requirements: "",
    language: "en",
    status: "running",
    session_id: 11,
    report: "",
    entries_created: 0,
    entries: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  }
}

function checkSession(status: string) {
  return conceptionSession({
    id: 11,
    role: "researcher",
    kind: "research",
    research_task_kind: "preparation",
    status: status as SessionDto["status"],
  })
}

describe("knowledge-check states before conception", () => {
  it("shows the open check stream, task link, and a confirmed Start conception now", async () => {
    const user = userEvent.setup()
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true)
    const task = preparationTask()
    state.topics = [task]
    state.checkSnapshot = {
      session: checkSession("running"),
      messages: [
        {
          message: {
            id: 1,
            seq: 0,
            role: "assistant",
            kind: "assistant",
            blocks: [{ type: "text", text: "the check is inventorying the scopes" }],
            stop_reason: null,
            usage: null,
            created_at: "2026-01-01T00:00:00Z",
          },
          tool_calls: [],
        },
      ],
      system_prompt: "",
      tool_names: [],
    }

    renderStudio()
    expect(screen.getByText("Knowledge check")).toBeInTheDocument()
    expect(screen.getByText("Running")).toBeInTheDocument()
    const stream = screen.getByTestId("stream")
    expect(stream).toBeInTheDocument()
    const wrapperClasses = stream.parentElement?.className.split(/\s+/) ?? []
    expect(wrapperClasses).toContain("flex")
    expect(wrapperClasses).toContain("flex-col")
    expect(wrapperClasses).toContain("h-[24rem]")
    expect(wrapperClasses).toContain("overflow-hidden")
    expect(screen.getByText("the check is inventorying the scopes")).toBeInTheDocument()
    expect(screen.getByRole("link", { name: "Open the check" })).toHaveAttribute(
      "href",
      "/research/21",
    )
    await user.click(screen.getByRole("button", { name: "Start conception now" }))
    expect(screen.queryByRole("button", { name: "Continue" })).not.toBeInTheDocument()
    expect(confirm).toHaveBeenCalledTimes(1)
    expect(startConception).toHaveBeenCalledTimes(1)
    confirm.mockRestore()
  })

  it("shows the report, entries and Start conception when the check is done", () => {
    state.topics = [
      preparationTask({ status: "done", session_id: null, report: "The report", entries_created: 3 }),
    ]
    renderStudio()
    expect(screen.getByText("The report")).toBeInTheDocument()
    expect(screen.getByText("Entries created: 3")).toBeInTheDocument()
    expect(screen.getByRole("link", { name: "Open Book › Knowledge" })).toHaveAttribute(
      "href",
      "/books/1/knowledge",
    )
    expect(screen.getByRole("button", { name: "Start conception" })).toBeInTheDocument()
  })

  it("shows the failed status and Start conception", () => {
    state.topics = [preparationTask({ status: "failed", session_id: null })]
    renderStudio()
    expect(screen.getByText("Failed")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Start conception" })).toBeInTheDocument()
  })

  it("continues a failed check on its existing task", async () => {
    const user = userEvent.setup()
    state.topics = [preparationTask({ status: "failed", session_id: 11 })]
    renderStudio()
    expect(screen.getByRole("button", { name: "Continue" })).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Continue" }))
    expect(runResearchTask).toHaveBeenCalledTimes(1)
    expect(runResearchTask.mock.calls[0][0]).toBe(21)
  })

  it("localizes a cancelled check in both locales", async () => {
    if (!i18n.hasResourceBundle("zh-CN", "agent")) {
      i18n.addResourceBundle("zh-CN", "agent", zhAgent)
    }
    if (!i18n.hasResourceBundle("zh-CN", "book")) {
      i18n.addResourceBundle("zh-CN", "book", zhBook)
    }
    if (!i18n.hasResourceBundle("zh-CN", "chapter")) {
      i18n.addResourceBundle("zh-CN", "chapter", zhChapter)
    }
    state.topics = [preparationTask({ status: "cancelled", session_id: null })]
    const english = renderStudio()
    expect(screen.getAllByText("Cancelled").length).toBeGreaterThan(0)
    expect(screen.getByTitle("Cancelled")).toBeInTheDocument()
    english.unmount()
    await i18n.changeLanguage("zh-CN")
    const chinese = renderStudio()
    expect(screen.getAllByText("已取消").length).toBeGreaterThan(0)
    expect(screen.getByTitle("已取消")).toBeInTheDocument()
    chinese.unmount()
    await i18n.changeLanguage("en")
  })

  it("offers Start conception and Check knowledge first when there is no check", async () => {
    const user = userEvent.setup()
    renderStudio()
    expect(screen.getByText(/No conception session yet/)).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Check knowledge first" }))
    expect(startKnowledgeCheck).toHaveBeenCalledTimes(1)
  })

  it("swaps to the conception session on a real session.updated event", async () => {
    const client = makeClient()
    const view = renderStudio(client)
    expect(screen.getByText(/No conception session yet/)).toBeInTheDocument()
    const session = conceptionSession({ status: "idle" })
    state.sessions = [session]
    state.snapshot = { session, messages: [], system_prompt: "", tool_names: [] }
    applyServerEvent(client, "session.updated", {
      session: {
        id: session.id,
        book_id: 1,
        role: "conception",
        kind: "conception",
        chapter_id: null,
        status: "idle",
      },
    })
    await waitFor(() => {
      expect(screen.queryByText(/No conception session yet/)).not.toBeInTheDocument()
    })
    expect(view.getByText("Conception studio")).toBeInTheDocument()
  })
})

})
