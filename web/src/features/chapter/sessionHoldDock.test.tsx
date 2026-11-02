/**
 * The dock must appear on the live path, not only after a reload (D-141, D-144).
 *
 * The author's report was 「dock 栏在哪」 — "where is the dock?" — with a screenshot of a `held` writer
 * session whose agent panel told them to decide in the dock, and no dock anywhere on the chapter
 * page. The dock was not broken: it renders (verified in a real browser) whenever the page reads a
 * chapter whose `open_sessions` already says `held`. It was **unreachable live**, because
 *
 * - `ChapterDto.open_sessions` is the client's only handle on the chapter's writer session, cached
 *   under `keys.chapters.detail(chapterId)`;
 * - `useWriterState` subscribes to the session snapshot **only** when that session is `held`;
 * - the agent brings the session to `held` by itself (`auto_pause`, `pause_after_outline`) — there
 *   is no client mutation, only a `session.updated` event, and that handler refreshed the session
 *   snapshot without refreshing the chapter.
 *
 * So the cached `open_sessions[0].status` stayed `running`, `findHeldCall` returned `undefined`,
 * `usePendingDraft().view` was `undefined`, and both the dock and the merged preview were absent
 * until something else refetched the chapter. A reload "fixing" it is the signature of a stale
 * cache, and it is why `pendingWriteDock.test.tsx` (which mounts with `held` already in the
 * fixture) passed while the author saw nothing.
 *
 * This test drives the whole chain the way the app does: the real `ChapterPage`, the real query
 * caches, and `applyServerEvent` delivering the server's own `session.updated`. The chapter query
 * refetch returns `held`, so the only thing under test is whether the event caused that refetch.
 *
 * Fail-before: remove the `keys.chapters.detail(chapterId)` invalidation from
 * `applyServerEvent`'s `session.updated` case and the dock never appears — the pane keeps rendering
 * the confirmed empty draft forever.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor } from "@testing-library/react"
import { MemoryRouter, Route, Routes } from "react-router"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { ChapterContent } from "@/api/gen/ChapterContent"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { useChapterDoc } from "@/stores/chapterDoc"

const BOOK_ID = 3
const CHAPTER_ID = 3
const SESSION_ID = 11
const CALL_ID = 317

/** The staged write: one paragraph, reading as an addition to the (empty) draft. */
function stagedContent(): ChapterContent {
  return {
    paragraphs: [{ pid: "p1", text: "城门下，一碗饭。" }],
    next_pid: 2,
    tail_open: false,
  }
}

function writerSession(status: SessionDto["status"]): SessionDto {
  return {
    id: SESSION_ID,
    book_id: BOOK_ID,
    role: "writer",
    kind: "chapter_write",
    chapter_id: CHAPTER_ID,
    track_id: 1,
    research_task_id: null,
    chapter_number: 2,
    research_topic: null,
    model_label: "Demo model",
    status,
    phase: "drafting",
    granularity: "paragraph",
    auto_pause: true,
    pause_after_outline: true,
    pause_requested: false,
    held_tool_call_id: status === "held" ? CALL_ID : null,
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
  }
}

/** The chapter as the client first sees it: the agent is still writing, nothing is held. */
function chapterWith(status: SessionDto["status"]): ChapterDto {
  return {
    id: CHAPTER_ID,
    book_id: BOOK_ID,
    track_id: 1,
    number: 2,
    title: "城门一碗饭",
    status: "drafting",
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
    open_sessions: [writerSession(status)],
  }
}

/** The held `insert_text` call with the text D-139 stages but does not commit. */
function heldCall(): ToolCallDto {
  return {
    id: CALL_ID,
    message_id: 124,
    seq: 3,
    call_index: 0,
    name: "insert_text",
    input: { text: "城门下，一碗饭。" },
    input_text: "",
    status: "held",
    is_error: false,
    result_text: null,
    appendices: [],
    ui: { inserted: ["p1"], appended_to: null, tail_open: false, word_count: 8 },
    sealed: false,
    pending_content: { content: stagedContent(), expected_version: 0 },
    created_at: "2026-01-01T00:00:00Z",
    finished_at: null,
  }
}

/**
 * The server, behind the endpoints: the chapter endpoint answers `running` until the session is
 * held, then `held`. Every other call the page makes gets an empty, well-formed response.
 */
const server = vi.hoisted(() => ({
  status: "running" as "running" | "held",
  chapterFetches: 0,
  sessionFetches: 0,
}))

function chapterResponse(): ChapterDto {
  return chapterWith(server.status)
}

vi.mock("@/api/endpoints/chapters", () => ({
  getChapter: async () => {
    server.chapterFetches += 1
    return chapterResponse()
  },
  patchChapter: async () => chapterResponse(),
  patchContent: async () => ({
    version: 1,
    ops: [],
    word_count: 0,
    tail_open: true,
  }),
  putContentText: async () => ({
    version: 1,
    ops: [],
    word_count: 0,
    tail_open: true,
  }),
  submitChapter: async () => ({ warnings: [] }),
  approveChapter: async () => ({ warnings: [] }),
}))

vi.mock("@/api/endpoints/sessions", () => ({
  getSession: async () => {
    server.sessionFetches += 1
    return {
      session: writerSession(server.status),
      messages: [],
      tool_calls: server.status === "held" ? [heldCall()] : [],
      inbox: [],
      inbox_pending: 0,
      system_prompt: "",
      tool_names: [],
    }
  },
  listSessions: async () => [],
  resumeSession: async () => writerSession("running"),
  control: async () => writerSession("running"),
  patchSession: async () => writerSession("running"),
  sendSessionMessage: async () => writerSession("running"),
  answerSession: async () => writerSession("running"),
  startDebugSession: async () => ({ warnings: [] }),
  getMessages: async () => ({ messages: [], next_cursor: null }),
}))

vi.mock("@/api/endpoints/reviews", () => ({
  listReviews: async () => ({ reports: [], feedback: [] }),
  startReview: async () => ({ session: writerSession("running") }),
  rereview: async () => ({ session: writerSession("running") }),
  sendBack: async () => ({ session: writerSession("running") }),
  maintainChapter: async () => ({ session: writerSession("running") }),
  resyncChapter: async () => ({ session: writerSession("running") }),
  listProposals: async () => [],
  acceptProposal: async () => null,
  rejectProposal: async () => null,
}))

vi.mock("@/api/endpoints/books", () => ({
  getBook: async () => ({
    id: BOOK_ID,
    title: "带着蓝色大肥鱼穿洪武",
    writing_language: "zh-CN",
    status: "active",
    settings: {
      granularity: "paragraph",
      auto_pause: true,
      pause_after_outline: true,
      target_words_min: 3000,
      target_words_max: 4000,
      recent_summary_count: 5,
      recent_chapter_count: 2,
      midterm_window: 10,
      auto_maintenance: true,
      canon_systems: { entities: true, timeline: true, memory: true, foreshadowing: true, goals: true },
    },
  }),
  listBooks: async () => [],
  patchBook: async () => null,
  getOverview: async () => null,
}))

vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => () => {},
  useWarningToasts: () => () => {},
}))

vi.mock("@/features/chapter/HistoryTab", () => ({ HistoryTab: () => null }))
vi.mock("@/features/chapter/ReviewTab", () => ({ ReviewTab: () => null }))

import ChapterPage from "@/features/chapter/ChapterPage"
import { applyServerEvent } from "@/api/events"

function renderPage(client: QueryClient) {
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[`/books/${BOOK_ID}/chapters/${CHAPTER_ID}`]}>
        <Routes>
          <Route
            path="/books/:bookId/chapters/:chapterId"
            element={<ChapterPage />}
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  useChapterDoc.getState().reset()
  server.status = "running"
  server.chapterFetches = 0
  server.sessionFetches = 0
})

describe("the held write's dock on the live path", () => {
  it("appears when a session.updated event makes the chapter's writer held, without a reload", async () => {
    const client = new QueryClient({
      defaultOptions: {
        queries: { retry: false, staleTime: 5_000, gcTime: Infinity },
      },
    })
    renderPage(client)

    // The page opened while the agent was still writing: no hold, so no dock and no merged pane.
    await waitFor(() => expect(server.chapterFetches).toBe(1))
    await screen.findByText("城门一碗饭")
    expect(screen.queryByTestId("pending-write-dock")).not.toBeInTheDocument()
    expect(server.sessionFetches).toBe(0)

    // The agent's write tool hits `auto_pause`: the server commits `held` and publishes the event.
    server.status = "held"
    applyServerEvent(client, "session.updated", {
      session: writerSession("held"),
      warnings: [],
    })

    // The invalidation refetches the chapter, `open_sessions[0]` now says `held`, `useWriterState`
    // subscribes to the snapshot, `findHeldCall` resolves the staged write, and the dock renders —
    // on the same page, with no reload and no other interaction.
    const dock = await screen.findByTestId("pending-write-dock")
    expect(dock).toHaveAttribute("data-standing", "current")
    expect(await screen.findByText("The agent wrote this. Apply it?")).toBeInTheDocument()
    expect(server.chapterFetches).toBe(2)
    expect(server.sessionFetches).toBeGreaterThan(0)
  })

  it("renders the merged preview beside the dock, so the decision is made on the text", async () => {
    const client = new QueryClient({
      defaultOptions: {
        queries: { retry: false, staleTime: 5_000, gcTime: Infinity },
      },
    })
    renderPage(client)
    await waitFor(() => expect(server.chapterFetches).toBe(1))
    await screen.findByText("城门一碗饭")

    server.status = "held"
    applyServerEvent(client, "session.updated", {
      session: writerSession("held"),
      warnings: [],
    })

    // The merged document is the staged one, marked as an addition rather than as the draft's text
    // (D-141): the author decides on the shape the chapter will have.
    const paragraph = await screen.findByText("城门下，一碗饭。")
    expect(paragraph).toHaveAttribute("data-kind", "added")
    expect(
      screen.getByText(
        "Preview: the highlighted paragraphs are the agent's text, not part of your draft yet.",
      ),
    ).toBeInTheDocument()
  })
})
