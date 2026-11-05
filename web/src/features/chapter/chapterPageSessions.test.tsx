/** ChapterPage's session focus/pin path (R3-06, R3-08). */
// The store reads the persisted language from i18n; import i18n first (the same cycle note as
// overview.test.tsx).
import "@/i18n"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { Outlet, RouterProvider, createMemoryRouter } from "react-router"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { BookDto } from "@/api/gen/BookDto"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import type { BookShell } from "@/features/workspace/shell"
import { keys } from "@/api/keys"
import { useUiStore } from "@/stores/ui"

const BOOK_ID = 3
const CHAPTER_ID = 3
const PLANNER_ID = 11

const server = vi.hoisted(() => ({
  chapter: null as ChapterDto | null,
  chapters: {} as Record<number, ChapterDto>,
  expanded: 0,
  patchCalls: [] as { chapterId: number; input: unknown }[],
  resumes: [] as { sessionId: number; feedback?: string }[],
  sessions: {} as Record<number, { session: SessionDto; tool_calls: ToolCallDto[] }>,
  pendingDraft: null as unknown,
}))

function planner(status: SessionDto["status"] = "running"): SessionDto {
  return {
    id: PLANNER_ID,
    book_id: BOOK_ID,
    role: "planner",
    kind: "chapter_outline",
    chapter_id: CHAPTER_ID,
    track_id: 1,
    research_task_id: null,
    chapter_number: 1,
    research_topic: null,
    model_label: "Demo",
    context_window: 100000,
    status,
    phase: "none",
    granularity: null,
    auto_pause: false,
    pause_after_outline: true,
    pause_requested: false,
    pause_after_step: false,
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
      research_budget_noticed: false,
    },
    parent_session_id: null,
    child_session_id: null,
    retrying: null,
    warnings: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    finished_at: null,
  }
}

function chapter(openSessions: SessionDto[]): ChapterDto {
  return chapterAt(CHAPTER_ID, "第一章", openSessions)
}

function chapterAt(id: number, title: string, openSessions: SessionDto[]): ChapterDto {
  return {
    id,
    book_id: BOOK_ID,
    track_id: 1,
    number: id,
    title,
    status: "planned",
    canon_status: "none",
    word_count: 0,
    has_outline: false,
    updated_at: "2026-01-01T00:00:00Z",
    guidance: "",
    outline: "",
    reference_notes: "",
    summary: "",
    content: { paragraphs: [], next_pid: 1, tail_open: true },
    content_version: 0,
    official_version_id: null,
    open_sessions: openSessions,
  }
}

const BOOK = {
  id: BOOK_ID,
  title: "Book",
  idea: "",
  genre: "",
  writing_language: "zh-CN",
  status: "active",
  settings: {
    granularity: "paragraph",
    auto_pause: false,
    pause_after_outline: true,
    target_words_min: 3000,
    target_words_max: 4000,
    recent_summary_count: 5,
    midterm_window: 10,
    auto_maintenance: true,
    canon_systems: { entities: true, timeline: true, memory: true, foreshadowing: true, goals: true },
  },
  model_overrides: {},
  conception: {},
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
} as unknown as BookDto

vi.mock("@/api/endpoints/chapters", () => ({
  getChapter: async (chapterId: number) => server.chapters[chapterId] ?? server.chapter,
  patchChapter: async () => server.chapter,
  patchContent: async (chapterId: number, input: unknown) => {
    server.patchCalls.push({ chapterId, input })
    return { version: 1, ops: [], word_count: 0, tail_open: true }
  },
  putContentText: async () => ({ version: 1, ops: [], word_count: 0, tail_open: true }),
  submitChapter: async () => ({ warnings: [] }),
  approveChapter: async () => ({ warnings: [] }),
  writeOutline: async () => ({ session_id: 2, warnings: [] }),
  writeChapter: async () => ({ session_id: 2, warnings: [] }),
}))
vi.mock("@/api/endpoints/sessions", () => ({
  getSession: async (sessionId: number) =>
    server.sessions[sessionId] ?? {
      session: planner(),
      messages: [],
      tool_calls: [],
      inbox: [],
      inbox_pending: 0,
      system_prompt: "",
      tool_names: [],
    },
  listSessions: async () => [],
  resumeSession: async (sessionId: number, input?: { feedback?: string }) => {
    server.resumes.push({ sessionId, ...input })
    return planner()
  },
  control: async () => planner(),
  patchSession: async () => planner(),
  sendSessionMessage: async () => planner(),
  answerSession: async () => planner(),
  startDebugSession: async () => ({ warnings: [] }),
  getMessages: async () => ({ messages: [], next_cursor: null }),
}))
vi.mock("@/api/endpoints/reviews", () => ({
  listReviews: async () => ({ reports: [], feedback: [] }),
  startReview: async () => ({ session: planner() }),
  rereview: async () => ({ session: planner() }),
  sendBack: async () => ({ session: planner() }),
  maintainChapter: async () => ({ session: planner() }),
  resyncChapter: async () => ({ session: planner() }),
  listProposals: async () => [],
  acceptProposal: async () => null,
  rejectProposal: async () => null,
}))
vi.mock("@/api/endpoints/books", () => ({
  getBook: async () => BOOK,
  listBooks: async () => [],
  patchBook: async () => BOOK,
  getOverview: async () => null,
}))
vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => () => {},
  useWarningToasts: () => () => {},
}))
vi.mock("@/features/chapter/usePendingDraft", () => ({
  usePendingDraft: () =>
    server.pendingDraft ?? {
      writer: { held: undefined, session: undefined, heldPids: [], heldWrite: false },
      view: undefined,
    },
}))
vi.mock("@/features/chapter/ReviewTab", () => ({ ReviewTab: () => null }))
vi.mock("@/features/chapter/HistoryTab", () => ({ HistoryTab: () => null }))

import ChapterPage from "@/features/chapter/ChapterPage"
import { pickPinnedSession, pickSession } from "@/features/agent/AgentPanel"

function Shell() {
  const shell = {
    book: BOOK,
    agentPanelCollapsed: true,
    expandAgentPanel: () => {
      server.expanded += 1
    },
  } as unknown as BookShell
  return <Outlet context={shell} />
}

function chapterText(id: number, title: string, text: string, openSessions: SessionDto[] = []) {
  const value = chapterAt(id, title, openSessions)
  value.content = { paragraphs: [{ pid: "p1", text }], next_pid: 2, tail_open: false }
  return value
}


function renderPage(tab = "outline", fixedHeight?: number) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const router = createMemoryRouter(
    [
      {
        element: <Shell />,
        children: [
          { path: "/books/:bookId/chapters/:chapterId", element: <ChapterPage /> },
        ],
      },
    ],
    { initialEntries: [`/books/${BOOK_ID}/chapters/${CHAPTER_ID}?tab=${tab}`] },
  )
  const page = (
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  )
  const rendered = render(
    fixedHeight === undefined ? (
      page
    ) : (
      <div style={{ height: `${fixedHeight}px`, display: "flex", flexDirection: "column" }}>{page}</div>
    ),
  )
  return { router, client, container: rendered.container }
}

beforeEach(() => {
  server.expanded = 0
  server.patchCalls = []
  server.resumes = []
  server.sessions = {}
  server.pendingDraft = null
  server.chapter = chapter([planner()])
  server.chapters = {}
  useUiStore.setState({ pinnedSession: {} })
})

describe("ChapterPage draft layout", () => {
  it("lets the draft column fill a fixed-height page instead of collapsing to content", async () => {
    server.chapter = chapterText(CHAPTER_ID, "第一章", "A 的正文")
    server.chapters[CHAPTER_ID] = chapterText(CHAPTER_ID, "第一章", "A 的正文")
    const { container } = renderPage("text", 900)
    await screen.findByText("A 的正文", {}, { timeout: 3000 })
    const root = container.querySelector(":scope > div > div") as HTMLElement
    expect(root.className).toContain("flex-1")
    expect(root.className).toContain("min-h-0")
    expect(root.className).toContain("flex-col")
    const tabs = root.querySelector(":scope > [data-slot=tabs]") as HTMLElement
    expect(tabs.className).toContain("flex")
    expect(tabs.className).toContain("min-h-0")
    expect(tabs.className).toContain("flex-1")
    const panel = tabs.querySelector(":scope > [data-slot=tabs-content]") as HTMLElement
    expect(panel.className).toContain("min-h-0")
    expect(panel.className).toContain("flex-1")
    expect(panel.className).toContain("flex-col")
    const region = panel.querySelector("[data-draft-region]") as HTMLElement
    expect(region.className).toContain("min-h-0")
    expect(region.className).toContain("flex-1")
    expect(region.className).toContain("flex-col")
    const scroll = region.querySelector(":scope > div") as HTMLElement
    expect(scroll.className).toContain("overflow-auto")
    expect(scroll.className).toContain("flex-1")
    expect(scroll.className).toContain("min-h-0")
  })
})

describe("ChapterPage session focus", () => {

  it("drops an in-progress paragraph editor when the chapter changes", async () => {
    const user = userEvent.setup()
    server.chapter = chapterText(CHAPTER_ID, "第一章", "A 的正文")
    server.chapters[CHAPTER_ID] = chapterText(CHAPTER_ID, "第一章", "A 的正文")
    const { client } = renderPage("text")
    expect(await screen.findByText("A 的正文", {}, { timeout: 3000 })).toBeInTheDocument()
    await user.click(screen.getByText("A 的正文"))
    const editor = screen.getByRole("textbox")
    await user.clear(editor)
    await user.type(editor, "A 的草稿")
    server.chapters[CHAPTER_ID] = chapterText(4, "第二章", "B 的正文")
    await act(async () => {
      await client.invalidateQueries({ queryKey: keys.chapters.detail(CHAPTER_ID) })
    })
    expect(await screen.findByText("B 的正文", {}, { timeout: 3000 })).toBeInTheDocument()
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument()
    expect(server.patchCalls).toHaveLength(0)
  })

  it("reopens the agent panel from the Outline tab's focus link", async () => {
    const user = userEvent.setup()
    renderPage()
    expect(await screen.findByText("第一章", {}, { timeout: 3000 })).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Focus in the agent panel" }))
    expect(server.expanded).toBe(1)
    expect(useUiStore.getState().pinnedSession[BOOK_ID]).toBe(PLANNER_ID)
  })

  it("remounts the header when the chapter id changes in place", async () => {
    const user = userEvent.setup()
    server.chapter = chapter([])
    server.chapters[CHAPTER_ID] = chapter([])
    const { client } = renderPage()
    expect(await screen.findByText("第一章", {}, { timeout: 3000 })).toBeInTheDocument()
    await user.click(screen.getAllByRole("button", { name: "Write the outline" })[0])
    expect(screen.getByText("Write the chapter outline")).toBeInTheDocument()
    // The query key stays the same (same route); only the returned row's id changes. A keyed
    // ChapterHeader must remount and close the dialog (R3-02/T-01).
    server.chapters[CHAPTER_ID] = chapterAt(4, "第二章", [])
    await act(async () => {
      await client.invalidateQueries({ queryKey: keys.chapters.detail(CHAPTER_ID) })
    })
    expect(await screen.findByText("第二章", {}, { timeout: 3000 })).toBeInTheDocument()
    expect(screen.queryByText("Write the chapter outline")).not.toBeInTheDocument()
  })

  it("closes the previous chapter's dialog when the chapter changes", async () => {
    const user = userEvent.setup()
    server.chapter = chapter([])
    server.chapters[CHAPTER_ID] = chapter([])
    server.chapters[4] = chapterAt(4, "第二章", [])
    const { router } = renderPage()
    expect(await screen.findByText("第一章", {}, { timeout: 3000 })).toBeInTheDocument()
    await user.click(screen.getAllByRole("button", { name: "Write the outline" })[0])
    expect(screen.getByText("Write the chapter outline")).toBeInTheDocument()
    await act(async () => {
      await router.navigate(`/books/${BOOK_ID}/chapters/4?tab=outline`)
    })
    expect(await screen.findByText("第二章", {}, { timeout: 3000 })).toBeInTheDocument()
    expect(screen.queryByText("Write the chapter outline")).not.toBeInTheDocument()
  })

  it("pins a session started from the Write-outline dialog instead of the oldest open one", async () => {
    const user = userEvent.setup()
    server.chapter = chapter([])
    renderPage()
    expect(await screen.findByText("第一章", {}, { timeout: 3000 })).toBeInTheDocument()
    await user.click(screen.getAllByRole("button", { name: "Write the outline" })[0])
    await user.click(screen.getByRole("button", { name: "Start planning" }))
    await waitFor(() => expect(useUiStore.getState().pinnedSession[BOOK_ID]).toBe(2))
    expect(server.expanded).toBe(1)
    // The pinned session wins over the older reviewer/planner candidates.
    const reviewer = planner("held")
    reviewer.id = 1
    reviewer.role = "reviewer"
    reviewer.kind = "chapter_review"
    const started = planner("running")
    started.id = 2
    expect(pickSession([reviewer, started], started, 2)?.id).toBe(2)
    // The chapter's own open sessions are searched first; the newly started planner wins even
    // when the book-wide session query has not caught up with it yet.
    expect(pickPinnedSession([reviewer, started], [], 2)?.id).toBe(2)
  })
})
