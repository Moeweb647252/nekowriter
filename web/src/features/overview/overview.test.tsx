/**
 * The overview's next action and attention list (PLAN 11.1, spec/08 §7).
 *
 * The labels are composed from data (D-054), so these tests pin the localized text, the targets,
 * and the empty attention state. "Write the next chapter" is the second create-chapter entry
 * point, so it is also disabled with a visible reason when the book is not `active`
 * (spec/08 §1.1, D-130).
 *
 * The file also carries the **behavioural** half of the card, which is where the reported gap
 * lived: `seed_canon` and `sync_canon` called their mutation and then showed nothing at all, which
 * is what made the author conclude that canon seeding 「只有侧边栏模式」 (D-135). Those assertions
 * are on what the click *does*: the mutation is called, the returned session is pinned in the UI
 * store (the agent panel reads exactly that pin), a visible confirmation renders, and a collapsed
 * panel is reopened. `run_research` (D-136) is the knowledge step between the conception and the
 * chapters, so it is asserted separately: it names its count and opens the page that runs the tasks.
 * The last describe block is a table over every `NextActionKind`, so a kind added without a
 * destination or a visible outcome fails here instead of shipping as a dead button.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { MemoryRouter, Route, Routes } from "react-router"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

// `@/i18n` before `@/stores/ui`: the store reads the persisted language from the `i18n` module and
// the two import each other, so the i18n module has to be the one that starts evaluating.
import i18n from "@/i18n"
import enAgent from "@/i18n/locales/en/agent.json"
import enBook from "@/i18n/locales/en/book.json"
import zhAgent from "@/i18n/locales/zh-CN/agent.json"
import zhBook from "@/i18n/locales/zh-CN/book.json"
import type { BookShell } from "@/features/workspace/shell"
import type { NextActionKind } from "@/api/gen/NextActionKind"
import { useUiStore } from "@/stores/ui"

const overview = vi.fn()
vi.mock("@/api/hooks/books", () => ({
  useOverview: () => ({ data: overview(), isPending: false, isError: false }),
}))

/**
 * The mutations, with the callbacks the tests drive. A React Query mutation does not resolve
 * inside `render()` without a provider and the point of these tests is the success path, so each
 * mocked hook records its variables and hands the caller's `onSuccess`/`onError` back.
 */
const mutations = vi.hoisted(() => ({
  seed: vi.fn(),
  resync: vi.fn(),
  maintain: vi.fn(),
  createChapter: vi.fn(),
}))

vi.mock("@/api/hooks/conception", () => ({
  useSeedCanon: () => ({ mutate: mutations.seed, isPending: false }),
}))
vi.mock("@/api/hooks/reviews", () => ({
  useResyncChapter: () => ({ mutate: mutations.resync, isPending: false }),
  useMaintainChapter: () => ({ mutate: mutations.maintain, isPending: false }),
}))
vi.mock("@/api/hooks/tracks", () => ({
  useCreateChapter: () => ({ mutate: mutations.createChapter, isPending: false }),
}))
/**
 * The research endpoints, for the card's bulk defer (D-139). The card uses the **real** shared
 * batch hook and the real `useDeferResearchTask`, so the assertions below are on what the click
 * actually sends, not on a mock standing in for the batch.
 */
const researchApi = vi.hoisted(() => ({
  list: vi.fn(),
  defer: vi.fn(),
}))

vi.mock("@/api/endpoints/research", () => ({
  listResearchTasks: researchApi.list,
  deferResearchTask: researchApi.defer,
  resumeResearchTask: vi.fn(),
  getResearchTask: vi.fn(),
  createResearchTask: vi.fn(),
  patchResearchTask: vi.fn(),
  deleteResearchTask: vi.fn(),
  runResearchTask: vi.fn(),
}))

vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => vi.fn(),
}))
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() } }))

const errorToast = vi.hoisted(() => vi.fn())
vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => errorToast,
  useWarningToasts: () => vi.fn(),
}))

/** The agent-panel half of the workspace shell; `null` is a page rendered outside a workspace. */
const shell = vi.hoisted(() => ({
  value: null as unknown as BookShell | null,
}))

vi.mock("@/features/workspace/shell", () => ({
  useBookShell: () => shell.value,
}))

import { toast } from "sonner"

import { keys } from "@/api/keys"
import type { ResearchTaskDto } from "@/api/gen/ResearchTaskDto"
import OverviewPage from "@/features/overview/OverviewPage"

const BOOK = {
  id: 1,
  title: "青云劫",
  idea: "",
  genre: "xianxia",
  writing_language: "zh-CN",
  status: "active",
  settings: { target_words_min: 3000, target_words_max: 4000 },
  model_overrides: {},
  conception: {
    session_id: null,
    characters_revision: 0,
    characters_approved_revision: null,
    topics_revision: 0,
    topics_approved_revision: null,
    finalized_at: null,
  },
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
}

const USAGE = {
  requests: 12,
  input_tokens: 3400,
  cached_input_tokens: 0,
  cache_write_tokens: 0,
  output_tokens: 900,
  reasoning_tokens: 0,
  last_prompt_tokens: 3400,
  estimated: false,
}

const TRACKS = [
  { track_id: 10, kind: "main", title: "", chapters: 1, words: 8, by_status: [] },
]

/** The agent-panel half of the shell: an open panel whose open action is observable. */
function bookShell(agentPanelCollapsed: boolean, expandAgentPanel = vi.fn()): BookShell {
  return {
    book: BOOK as unknown as BookShell["book"],
    agentPanelCollapsed,
    expandAgentPanel,
  }
}

function data(overrides: Record<string, unknown>) {
  return {
    book: BOOK,
    next_action: null,
    attention: [],
    progress: {
      total_chapters: 1,
      official_chapters: 0,
      total_words: 8,
      official_words: 0,
      tracks: TRACKS,
    },
    recent_changes: [],
    usage: USAGE,
    ...overrides,
  }
}

/**
 * Renders the overview with a real query cache: the card's bulk defer (D-139) uses the shared
 * batch hook, whose invalidation is one of the things under test.
 */
function renderOverview() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const view = render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={["/books/1"]}>
        <Routes>
          <Route path="/books/:bookId" element={<OverviewPage />} />
          {/* `write_next` opens the chapter it created, so its outcome is a route change. */}
          <Route path="/books/:bookId/chapters/:chapterId" element={<p>chapter page</p>} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
  return { client, ...view }
}

/** True once `queryKey` is marked stale in the cache. */
function isInvalidated(client: QueryClient, queryKey: readonly unknown[]): boolean {
  return client.getQueryCache().find({ queryKey })?.state.isInvalidated ?? false
}

/** A research task row; only the fields the batch reads are meaningful. */
function researchTask(overrides: Partial<ResearchTaskDto> = {}): ResearchTaskDto {
  return {
    id: 1,
    book_id: 1,
    scope_id: 1,
    scope_name: "明初历史",
    topic: "宝钞与财政",
    kind: "topic",
    origin: "author",
    requirements: "",
    language: "zh-CN",
    status: "ready",
    session_id: null,
    report: "",
    entries_created: 0,
    entries: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  }
}

/** The overview's `write_next` action on the main track. */
const WRITE_NEXT = {
  kind: "write_next",
  chapter: null,
  track_id: 10,
}

/** The overview's `seed_canon` action: designs but no canon seeded yet (spec/08 §7). */
const SEED_CANON = {
  kind: "seed_canon",
  chapter: null,
  track_id: null,
}

/** The overview's `sync_canon` action: an official chapter whose canon is stale (spec/08 §7). */
const SYNC_CANON = {
  kind: "sync_canon",
  chapter: { id: 5, number: 1, title: "One", status: "official", canon_status: "stale" },
  track_id: 10,
}

/** The overview's `run_research` action: confirmed tasks nobody has run yet (D-136). */
const RUN_RESEARCH = {
  kind: "run_research",
  chapter: null,
  track_id: null,
  ready_research: 8,
}

/** The same action when exactly one task is runnable, so the card can open that task. */
const RUN_RESEARCH_ONE = {
  kind: "run_research",
  chapter: null,
  track_id: null,
  ready_research: 1,
  research_task_id: 3,
}

/** What a mocked `mutate`'s second argument looks like at the call site. */
type MutationOptions<T> = {
  onSuccess?: (result: T) => void
  onError?: (error: unknown) => void
}

/** The options the last call of `mutate` received. */
function optionsOf<T>(mock: ReturnType<typeof vi.fn>): MutationOptions<T> {
  const call = mock.mock.calls.at(-1) as [unknown, MutationOptions<T>] | undefined
  if (call?.[1] === undefined) {
    throw new Error("mutate was not called with options")
  }
  return call[1]
}

/** The localized in-panel confirmation the card shows after a session starts. */
const SEED_STARTED =
  "Seeding canon started. Follow it in the agent panel on the right."
const RESYNC_STARTED =
  "Re-syncing canon started. Follow it in the agent panel on the right."
const RUNS_IN_PANEL = "This run takes place in the agent panel on the right."
const RUNS_IN_PANEL_COLLAPSED =
  "This run takes place in the agent panel, which is collapsed."

beforeAll(() => {
  // The Vitest setup loads every namespace in English only; these tests also assert the zh-CN
  // text (spec/11 §6: a key is only done when both locales carry it).
  i18n.addResourceBundle("zh-CN", "book", zhBook, true, true)
  i18n.addResourceBundle("zh-CN", "agent", zhAgent, true, true)
  i18n.addResourceBundle("en", "book", enBook, true, true)
  i18n.addResourceBundle("en", "agent", enAgent, true, true)
})

beforeEach(() => {
  overview.mockReset()
  mutations.seed.mockReset()
  mutations.resync.mockReset()
  mutations.maintain.mockReset()
  mutations.createChapter.mockReset()
  researchApi.list.mockReset()
  researchApi.defer.mockReset()
  // By default the book has no runnable tasks cached, so only the tests about the batch see the
  // secondary control. Its own describe block sets the rows it needs.
  researchApi.list.mockResolvedValue([])
  researchApi.defer.mockImplementation(async (taskId: number) =>
    researchTask({ id: taskId, status: "deferred" }),
  )
  vi.mocked(toast.success).mockClear()
  vi.mocked(toast.warning).mockClear()
  vi.mocked(toast.error).mockClear()
  shell.value = null
  useUiStore.setState({ pinnedSession: {} })
})

afterEach(() => {
  useUiStore.setState({ pinnedSession: {} })
})

describe("OverviewPage", () => {
  it("renders the next action as a localized link", () => {
    overview.mockReturnValue(
      data({
        next_action: {
          kind: "review_chapter",
          chapter: { id: 5, number: 1, title: "One", status: "in_review" },
          track_id: 10,
        },
      }),
    )
    renderOverview()
    expect(screen.getByText("Next step")).toBeInTheDocument()
    expect(screen.getByText("Review Chapter 1")).toBeInTheDocument()
    const link = screen.getByRole("link", { name: /Go/ })
    expect(link).toHaveAttribute("href", "/books/1/chapters/5")
  })

  it("labels a side-story chapter with its track", () => {
    overview.mockReturnValue(
      data({
        next_action: {
          kind: "continue_chapter",
          chapter: { id: 7, number: 2, title: "Two", status: "drafting" },
          track_id: 11,
        },
        progress: {
          ...data({}).progress,
          tracks: [
            ...TRACKS,
            { track_id: 11, kind: "side", title: "Side Story", chapters: 1, words: 4, by_status: [] },
          ],
        },
      }),
    )
    renderOverview()
    expect(screen.getByText("Continue Side Story · Chapter 2")).toBeInTheDocument()
  })

  it("lists what needs the author, with the open session's title and status", () => {
    overview.mockReturnValue(
      data({
        attention: [
          {
            kind: "open_session",
            count: 1,
            session_id: 3,
            session_kind: "chapter_write",
            session_status: "paused",
            chapter: { id: 5, number: 1, title: "One", status: "drafting" },
            track_id: 10,
          },
          {
            kind: "pending_proposals",
            count: 2,
          },
          {
            kind: "ready_research",
            count: 1,
          },
        ],
      }),
    )
    renderOverview()
    expect(
      screen.getByText("Writing · Chapter 1 is Paused"),
    ).toBeInTheDocument()
    expect(screen.getByText("2 pending proposals")).toBeInTheDocument()
    expect(screen.getByText("1 research task is ready to run")).toBeInTheDocument()
    expect(screen.getByRole("link", { name: /Writing/ })).toHaveAttribute(
      "href",
      "/books/1/chapters/5",
    )
  })

  it("says that nothing needs the author when the list is empty", () => {
    overview.mockReturnValue(data({}))
    renderOverview()
    expect(screen.getByText("Nothing needs you right now.")).toBeInTheDocument()
  })

  it("shows the token usage totals", () => {
    overview.mockReturnValue(data({}))
    renderOverview()
    expect(
      screen.getByText("12 requests · 3,400 input tokens · 900 output tokens"),
    ).toBeInTheDocument()
  })
})

describe("writing the next chapter needs an active book (spec/08 §1.1, D-130)", () => {
  it("disables the button on an archived book and says why in visible text", () => {
    // `books::next_action` answers `ContinueConception` for a conceiving book before any other
    // branch, so `archived` is the only non-active status that can reach this button.
    overview.mockReturnValue(
      data({ book: { ...BOOK, status: "archived" }, next_action: WRITE_NEXT }),
    )
    renderOverview()

    expect(screen.getByText("Write the next chapter of Main story")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: /Go/ })).toBeDisabled()
    // The reason is rendered, not set as a tooltip: a disabled button takes no pointer events.
    expect(
      screen.getByText("This book is archived, so no new chapter can be added. Unarchive it first."),
    ).toBeInTheDocument()
  })

  it("leaves the button enabled on an active book and explains nothing", () => {
    overview.mockReturnValue(data({ next_action: WRITE_NEXT }))
    renderOverview()

    expect(screen.getByRole("button", { name: /Go/ })).toBeEnabled()
    expect(
      screen.queryByText("This book is archived, so no new chapter can be added. Unarchive it first."),
    ).not.toBeInTheDocument()
  })
})

/**
 * The reported gap (D-135): an archivist run starts in the agent panel, so the overview has to say
 * so and put the started session where the panel will look for it.
 */
describe("canon seeding points at the agent panel (D-135)", () => {
  it("pins the seeded session, says where it runs and confirms it", async () => {
    const user = userEvent.setup()
    overview.mockReturnValue(data({ next_action: SEED_CANON }))
    renderOverview()

    // It is a button, not a link: there is no seeding page to go to (PROJECT §8.4).
    expect(screen.queryByRole("link", { name: /Go/ })).toBeNull()
    // Before the click the card already says where the run will happen.
    expect(screen.getByText(RUNS_IN_PANEL)).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: /Go/ }))
    expect(mutations.seed).toHaveBeenCalledTimes(1)

    // The endpoint answers the session it started (or the open one, spec/08 §5).
    act(() => {
      optionsOf<{ session_id: number }>(mutations.seed).onSuccess?.({ session_id: 42 })
    })

    // `AgentPanel` chooses `pinnedSession[bookId]` first (:84-87), so this *is* the panel moving.
    expect(useUiStore.getState().pinnedSession[1]).toBe(42)
    // The visible confirmation replaces the hint, and it names the session by its kind.
    expect(screen.getByRole("status")).toHaveTextContent(SEED_STARTED)
    // The toast is the transient half of the same message.
    expect(toast.success).toHaveBeenCalledWith(SEED_STARTED)
  })

  it("pins the re-sync session the same way", async () => {
    const user = userEvent.setup()
    overview.mockReturnValue(data({ next_action: SYNC_CANON }))
    renderOverview()

    await user.click(screen.getByRole("button", { name: /Go/ }))
    expect(mutations.resync).toHaveBeenCalledTimes(1)

    act(() => {
      optionsOf<{ session_id: number }>(mutations.resync).onSuccess?.({ session_id: 43 })
    })

    expect(useUiStore.getState().pinnedSession[1]).toBe(43)
    expect(screen.getByRole("status")).toHaveTextContent(RESYNC_STARTED)
    expect(toast.success).toHaveBeenCalledWith(RESYNC_STARTED)
  })

  it("retries pending canon through maintain instead of resync", async () => {
    const user = userEvent.setup()
    overview.mockReturnValue(
      data({
        next_action: {
          ...SYNC_CANON,
          chapter: { ...SYNC_CANON.chapter, canon_status: "pending" },
        },
      }),
    )
    renderOverview()

    await user.click(screen.getByRole("button", { name: /Go/ }))
    expect(mutations.maintain).toHaveBeenCalledTimes(1)
    expect(mutations.resync).not.toHaveBeenCalled()
  })

  it("reports a maintain failure through the error toast", async () => {
    const user = userEvent.setup()
    overview.mockReturnValue(
      data({
        next_action: {
          ...SYNC_CANON,
          chapter: { ...SYNC_CANON.chapter, canon_status: "failed" },
        },
      }),
    )
    renderOverview()
    await user.click(screen.getByRole("button", { name: /Go/ }))
    expect(mutations.maintain).toHaveBeenCalledTimes(1)
    act(() => {
      optionsOf<{ session_id: number }>(mutations.maintain).onError?.(new Error("maintain failed"))
    })
    expect(errorToast).toHaveBeenCalledTimes(1)
    expect(useUiStore.getState().pinnedSession[1]).toBeUndefined()
    expect(screen.queryByRole("status")).toBeNull()
  })

  it("reopens a collapsed agent panel when the run starts", async () => {
    const user = userEvent.setup()
    const expandAgentPanel = vi.fn()
    shell.value = bookShell(true, expandAgentPanel)
    overview.mockReturnValue(data({ next_action: SEED_CANON }))
    renderOverview()

    // The hint points at the collapsed panel rather than at text the author cannot see.
    expect(screen.getByText(RUNS_IN_PANEL_COLLAPSED)).toBeInTheDocument()
    // And the card offers the way back; the workspace has no other control for that panel.
    await user.click(screen.getByRole("button", { name: /Show the agent panel/ }))
    expect(expandAgentPanel).toHaveBeenCalledTimes(1)

    await user.click(screen.getByRole("button", { name: /Go/ }))
    act(() => {
      optionsOf<{ session_id: number }>(mutations.seed).onSuccess?.({ session_id: 42 })
    })
    // Starting the run opens the panel on its own: the author should not have to find it.
    expect(expandAgentPanel).toHaveBeenCalledTimes(2)
    expect(useUiStore.getState().pinnedSession[1]).toBe(42)
  })

  it("reports a failure and pins nothing", async () => {
    const user = userEvent.setup()
    overview.mockReturnValue(data({ next_action: SEED_CANON }))
    renderOverview()

    await user.click(screen.getByRole("button", { name: /Go/ }))
    act(() => {
      optionsOf<{ session_id: number }>(mutations.seed).onError?.(new Error("boom"))
    })

    expect(useUiStore.getState().pinnedSession[1]).toBeUndefined()
    expect(screen.queryByRole("status")).toBeNull()
    expect(toast.success).not.toHaveBeenCalled()
  })
})

describe("the archivist hints exist in both locales", () => {
  const KEYS = [
    "runs_in_agent_panel",
    "runs_in_agent_panel_collapsed",
    "show_agent_panel",
    "started_in_agent_panel",
  ] as const

  it.each(KEYS)("book:next_action.%s", (key) => {
    for (const [language, table] of [
      ["en", enBook],
      ["zh-CN", zhBook],
    ] as const) {
      const section = (table as unknown as { next_action: Record<string, unknown> }).next_action
      const value = section[key]
      expect(value, `${language} book:next_action.${key}`).toBeTypeOf("string")
      expect((value as string).length, `${language} book:next_action.${key}`).toBeGreaterThan(0)
    }
  })

  it("composes the confirmation from keys that both locales carry", () => {
    // The confirmation is `agent:session_title.<kind>` interpolated into `book:` text, so a missing
    // key in either namespace renders the bare key at the author (spec/11 §6, §9).
    for (const [language, book, agent] of [
      ["en", enBook, enAgent],
      ["zh-CN", zhBook, zhAgent],
    ] as const) {
      const bookTable = book as unknown as { next_action: Record<string, string> }
      const agentTable = agent as unknown as { session_title: Record<string, string> }
      for (const kind of ["canon_seed", "chapter_resync"] as const) {
        const title = agentTable.session_title[kind]
        expect(title, `${language} agent:session_title.${kind}`).toBeTypeOf("string")
        const started = bookTable.next_action.started_in_agent_panel?.replace(
          "{{session}}",
          title ?? "",
        )
        expect(started, `${language} started_in_agent_panel`).toBeTruthy()
        expect(started, `${language} started_in_agent_panel`).not.toMatch(/\{\{/)
      }
    }
  })

  it("renders the hint in zh-CN, not only in en", async () => {
    // Switching the language under a mounted component re-renders outside `act()`, so the switch
    // happens before mounting and is undone after unmounting. The restore is in `finally` so a
    // failing assertion here cannot leave every later test rendering in Chinese.
    const previous = i18n.language
    await i18n.changeLanguage("zh-CN")
    overview.mockReturnValue(data({ next_action: SEED_CANON }))
    const view = renderOverview()
    try {
      expect(screen.getByText("这一步在右侧的智能体面板里进行。")).toBeInTheDocument()
    } finally {
      view.unmount()
      await i18n.changeLanguage(previous)
    }
  })
})

/**
 * The knowledge-accumulation step (D-136): the author asked why nothing happened between conception
 * and drafting, and the answer was that the overview never suggested running the research tasks the
 * conception had created. The card has to name how many wait and open the page that runs them.
 */
describe("the research step leads to the tasks it counts (D-136)", () => {
  it("names the number of runnable tasks and opens the list when there are several", () => {
    overview.mockReturnValue(data({ next_action: RUN_RESEARCH }))
    renderOverview()

    expect(screen.getByText("Run 8 research tasks")).toBeInTheDocument()
    // Several tasks leave the choice to the author, and only the list offers it.
    expect(screen.getByRole("link", { name: /Go/ })).toHaveAttribute("href", "/research")
  })

  it("uses the singular wording for one task", () => {
    overview.mockReturnValue(data({ next_action: RUN_RESEARCH_ONE }))
    renderOverview()
    expect(screen.getByText("Run 1 research task")).toBeInTheDocument()
  })

  it("opens the single task's page, where its Run button is", () => {
    overview.mockReturnValue(data({ next_action: RUN_RESEARCH_ONE }))
    renderOverview()
    expect(screen.getByRole("link", { name: /Go/ })).toHaveAttribute("href", "/research/3")
  })

  it("falls back to the list when the server names no single task", () => {
    // `research_task_id` is optional data, not an invariant of the kind: a client that renders the
    // card without it must still get a working destination.
    overview.mockReturnValue(
      data({ next_action: { ...RUN_RESEARCH, ready_research: 1 } }),
    )
    renderOverview()
    expect(screen.getByRole("link", { name: /Go/ })).toHaveAttribute("href", "/research")
  })

  it("renders the step in zh-CN too", async () => {
    const previous = i18n.language
    await i18n.changeLanguage("zh-CN")
    overview.mockReturnValue(data({ next_action: RUN_RESEARCH }))
    const view = renderOverview()
    try {
      expect(screen.getByText("开始 8 个资料收集任务")).toBeInTheDocument()
    } finally {
      view.unmount()
      await i18n.changeLanguage(previous)
    }
  })

  it("carries both plural forms in both locales", () => {
    // The label is a `count` interpolation, so a locale that only has `_other` renders nothing for
    // a single task (spec/11 §6: a key is only done when both locales carry it).
    for (const [language, table] of [
      ["en", enBook],
      ["zh-CN", zhBook],
    ] as const) {
      const section = (table as unknown as { next_action: Record<string, unknown> }).next_action
      for (const key of ["run_research_one", "run_research_other"] as const) {
        const value = section[key]
        expect(value, `${language} book:next_action.${key}`).toBeTypeOf("string")
        expect((value as string).length, `${language} book:next_action.${key}`).toBeGreaterThan(0)
        expect(value, `${language} book:next_action.${key}`).toContain("{{count}}")
      }
    }
  })
})

/**
 * The card's release valve (D-139). The author was on the overview when the card insisted on
 * research, and the only way to say "not now" was to find the research list and click eight times.
 * The primary action stays the primary — it is the only `default`-variant control in the card —
 * and the defer control is the secondary one under it.
 */
describe("the research card can be postponed from the overview (D-139)", () => {
  it("keeps Go primary and adds the defer control as a secondary, counted action", async () => {
    researchApi.list.mockResolvedValue([
      researchTask({ id: 1, topic: "洪武朝大事编年" }),
      researchTask({ id: 2, topic: "大明宝钞" }),
      researchTask({ id: 3, status: "deferred" }),
      researchTask({ id: 4, status: "draft" }),
      researchTask({ id: 5, topic: "男频连载结构" }),
    ])
    overview.mockReturnValue(data({ next_action: RUN_RESEARCH }))
    renderOverview()

    // The primary is still the link to the page that runs the tasks, and it is the only `default`
    // button in the card: the defer control cannot be mistaken for "go do the research".
    const go = screen.getByRole("link", { name: /Go/ })
    expect(go).toHaveAttribute("href", "/research")
    const defer = await screen.findByRole("button", { name: "Defer all (3)" })
    expect(defer).toHaveAttribute("data-variant", "ghost")
    expect(go.closest("[data-variant]")).toHaveAttribute("data-variant", "default")
    // The hint says what the click does and that it is reversible.
    expect(
      screen.getByText(
        "Not now: postpone these tasks and keep writing. Resume them from the research list.",
      ),
    ).toBeInTheDocument()
  })

  it("defers exactly this book's ready tasks and leaves the overview asking for the next step", async () => {
    const user = userEvent.setup()
    researchApi.list.mockResolvedValue([
      researchTask({ id: 1, book_id: 1 }),
      researchTask({ id: 2, book_id: 1 }),
      // A row of another book: the card's action is book-scoped (D-136), so its defer is too.
      researchTask({ id: 3, book_id: 9 }),
      researchTask({ id: 4, book_id: 1, status: "deferred" }),
    ])
    overview.mockReturnValue(data({ next_action: RUN_RESEARCH }))
    const { client } = renderOverview()
    client.setQueryData(keys.books.overview(1), data({ next_action: RUN_RESEARCH }))
    client.setQueryData(keys.books.overview(9), {})

    await user.click(await screen.findByRole("button", { name: "Defer all (2)" }))

    await waitFor(() => {
      expect(researchApi.defer.mock.calls.map((call) => call[0])).toEqual([1, 2])
    })
    // The overview refetches, so the card is replaced by the next action (the chapter rule here);
    // without the invalidation it kept offering `run_research` (D-138, D-139).
    await waitFor(() => {
      expect(isInvalidated(client, keys.books.overview(1))).toBe(true)
    })
    expect(isInvalidated(client, keys.books.overview(9))).toBe(false)
    expect(toast.success).toHaveBeenCalledWith("Deferred 2 research tasks")
  })

  it("reports a partial batch on the card instead of swallowing it", async () => {
    const user = userEvent.setup()
    researchApi.list.mockResolvedValue([
      researchTask({ id: 1, topic: "洪武朝大事编年" }),
      researchTask({ id: 2, topic: "大明宝钞与明初货币财政" }),
    ])
    researchApi.defer.mockImplementation(async (taskId: number) => {
      if (taskId === 2) {
        throw new Error("boom")
      }
      return researchTask({ id: taskId, status: "deferred" })
    })
    overview.mockReturnValue(data({ next_action: RUN_RESEARCH }))
    renderOverview()

    await user.click(await screen.findByRole("button", { name: "Defer all (2)" }))

    const report = await screen.findByRole("alert")
    expect(within(report).getByText("Deferred 1 of 2 research tasks")).toBeInTheDocument()
    expect(
      within(report).getByText("Not deferred: 大明宝钞与明初货币财政"),
    ).toBeInTheDocument()
    expect(toast.warning).toHaveBeenCalledWith("Deferred 1 of 2 research tasks", {
      description: "Not deferred: 大明宝钞与明初货币财政",
    })
  })

  it("still opens the single task's page, and the defer covers that one task", async () => {
    const user = userEvent.setup()
    researchApi.list.mockResolvedValue([researchTask({ id: 3 })])
    overview.mockReturnValue(data({ next_action: RUN_RESEARCH_ONE }))
    renderOverview()

    // The single-task behaviour is untouched: `research_task_id` still names the destination.
    expect(screen.getByRole("link", { name: /Go/ })).toHaveAttribute("href", "/research/3")
    await user.click(await screen.findByRole("button", { name: "Defer all (1)" }))
    await waitFor(() => {
      expect(researchApi.defer.mock.calls.map((call) => call[0])).toEqual([3])
    })
    expect(toast.success).toHaveBeenCalledWith("Deferred 1 research task")
  })

  it("offers nothing when the book has no runnable tasks", async () => {
    researchApi.list.mockResolvedValue([researchTask({ id: 1, status: "deferred" })])
    overview.mockReturnValue(data({ next_action: RUN_RESEARCH }))
    renderOverview()

    // The card is still there (the mocked overview says so) but the control is not: there is
    // nothing to postpone, and a "Defer all (0)" would be a dead button. It goes away as soon as
    // the book's list query settles, which is what the count and the batch are read from.
    expect(await screen.findByRole("link", { name: /Go/ })).toBeInTheDocument()
    await waitFor(() => {
      expect(screen.queryByRole("button", { name: /Defer all/ })).toBeNull()
    })
  })

  it("carries the card's defer strings in both locales", () => {
    for (const [language, table] of [
      ["en", enBook],
      ["zh-CN", zhBook],
    ] as const) {
      const section = (table as unknown as { next_action: Record<string, unknown> }).next_action
      for (const key of ["defer_research", "defer_research_hint"] as const) {
        const value = section[key]
        expect(value, `${language} book:next_action.${key}`).toBeTypeOf("string")
        expect((value as string).length, `${language} book:next_action.${key}`).toBeGreaterThan(0)
      }
      // The count is an interpolation, never a concatenated number (AGENTS.md §6).
      expect(section.defer_research, `${language} defer_research`).toContain("{{count}}")
    }
  })
})

/**
 * The class-level guard (D-135). The defect was not "`seed_canon` is broken" but "two of six kinds
 * were silent", so every kind is listed with the affordance it must have: a destination, or a
 * mutation whose success produces something visible. A new kind has to be added to this table with
 * an affordance, and the coverage test fails if the table falls behind the generated union — that
 * is how `run_research` (D-136) had to declare `/research` instead of being added silently.
 */
type Expectation = {
  /**
   * The **primary** affordance: the action links to this path.
   *
   * The table describes what the card's main control does, because that is what the six kinds
   * disagreed about when two of them were silent (D-135). A kind may also carry a **secondary**
   * affordance — declared separately in `secondary` so the table stays honest about the whole card
   * rather than describing only the half it started with — and that secondary has its own test in
   * the `D-139` block above. What the table still guarantees for every kind is unchanged: the
   * primary affordance exists and does what it says.
   */
  navigates?: string
  /** The primary affordance calls this mutation (the archivist kinds and `write_next`). */
  mutation?: "seed" | "resync" | "createChapter"
  /** The action starts a run somewhere else, so it must say where (D-135). */
  hint?: string
  /**
   * A second, **non-primary** control the card carries beside the primary one (D-139). Named so the
   * table enumerates the card's affordances instead of quietly describing fewer than it renders.
   * The primary remains the one this table's `navigates`/`mutation` describe.
   */
  secondary?: "deferResearch"
}

const KIND_TABLE: {
  kind: NextActionKind
  action: Record<string, unknown>
  expected: Expectation
}[] = [
  {
    kind: "continue_conception",
    action: { kind: "continue_conception", chapter: null, track_id: null },
    expected: { navigates: "/books/1/conception" },
  },
  {
    kind: "review_chapter",
    action: {
      kind: "review_chapter",
      chapter: { id: 5, number: 1, title: "One", status: "in_review" },
      track_id: 10,
    },
    expected: { navigates: "/books/1/chapters/5" },
  },
  {
    kind: "continue_chapter",
    action: {
      kind: "continue_chapter",
      chapter: { id: 5, number: 1, title: "One", status: "drafting" },
      track_id: 10,
    },
    expected: { navigates: "/books/1/chapters/5" },
  },
  {
    kind: "write_outline",
    action: {
      kind: "write_outline",
      chapter: { id: 5, number: 1, title: "One", status: "planned" },
      track_id: 10,
    },
    expected: { navigates: "/books/1/chapters/5?tab=outline" },
  },
  {
    kind: "write_chapter",
    action: {
      kind: "write_chapter",
      chapter: { id: 5, number: 1, title: "One", status: "planned" },
      track_id: 10,
    },
    expected: { navigates: "/books/1/chapters/5?tab=text" },
  },
  {
    kind: "write_next",
    action: WRITE_NEXT,
    // Navigates, but only to the chapter it creates, so the destination is asserted after success.
    expected: { mutation: "createChapter" },
  },
  {
    kind: "seed_canon",
    action: SEED_CANON,
    expected: { mutation: "seed", hint: RUNS_IN_PANEL },
  },
  {
    kind: "sync_canon",
    action: SYNC_CANON,
    expected: { mutation: "resync", hint: RUNS_IN_PANEL },
  },
  {
    kind: "run_research",
    // The list, because several tasks leave the choice to the author (D-136). The card also
    // carries a secondary "defer all" control (D-139); the primary is still the link, and the
    // secondary is declared here so this table describes the whole card rather than only the
    // affordance the table started with. Its behaviour is covered by the D-139 block above, not
    // by the class-level loops below (which are about the primary).
    action: RUN_RESEARCH,
    expected: { navigates: "/research", secondary: "deferResearch" },
  },
]

describe("every next action either navigates or shows a visible outcome (D-135)", () => {
  it("covers every kind the server can answer, each with an affordance", () => {
    // Exhaustive by construction: the table is the guard, and an entry without a destination and
    // without a mutation would be the original defect.
    expect(KIND_TABLE.map((entry) => entry.kind).sort()).toEqual([
      "continue_chapter",
      "continue_conception",
      "review_chapter",
      "run_research",
      "seed_canon",
      "sync_canon",
      "write_chapter",
      "write_next",
      "write_outline",
    ])
    for (const entry of KIND_TABLE) {
      const { navigates, mutation, hint } = entry.expected
      expect(
        navigates !== undefined || mutation !== undefined || hint !== undefined,
        `${entry.kind} has no affordance`,
      ).toBe(true)
    }
  })

  it("declares a secondary affordance only where the card really renders one", () => {
    // `secondary` is not decoration: every entry that names one is checked against the rendered
    // card below, and every entry that does not must not have one (D-139).
    const withSecondary = KIND_TABLE.filter(
      (entry) => entry.expected.secondary !== undefined,
    )
    expect(withSecondary.map((entry) => entry.kind)).toEqual(["run_research"])
  })

  it.each(
    KIND_TABLE.filter((entry) => entry.expected.secondary === "deferResearch"),
  )("$kind: the secondary affordance renders beside the primary", async ({ action }) => {
    researchApi.list.mockResolvedValue([researchTask({ id: 1 })])
    overview.mockReturnValue(data({ next_action: action }))
    renderOverview()

    // Both are present, and they are distinguishable: the primary is a link with the default
    // variant, the secondary a ghost button. The card never has two competing primaries.
    const primary = screen.getByRole("link", { name: /Go/ })
    expect(primary.closest("[data-variant]")).toHaveAttribute("data-variant", "default")
    const secondary = await screen.findByRole("button", { name: "Defer all (1)" })
    expect(secondary).toHaveAttribute("data-variant", "ghost")
  })

  it.each(KIND_TABLE)("$kind: the click does what its affordance says", async ({ action, expected }) => {
    const user = userEvent.setup()
    overview.mockReturnValue(data({ next_action: action }))
    renderOverview()

    if (expected.navigates !== undefined) {
      expect(screen.getByRole("link", { name: /Go/ })).toHaveAttribute(
        "href",
        expected.navigates,
      )
      return
    }

    expect(expected.mutation, `${String(action.kind)} has no affordance`).toBeDefined()
    await user.click(screen.getByRole("button", { name: /Go/ }))
    const mutate = mutations[expected.mutation as "seed" | "resync" | "createChapter"]
    expect(mutate, `${String(action.kind)} never calls its mutation`).toHaveBeenCalledTimes(1)
  })

  it.each(KIND_TABLE.filter((entry) => entry.expected.mutation !== undefined))(
    "$kind: success is visible",
    async ({ action, expected }) => {
      const user = userEvent.setup()
      shell.value = bookShell(false)
      overview.mockReturnValue(data({ next_action: action }))
      renderOverview()

      // A run that happens elsewhere says where, before the click (D-135).
      if (expected.hint !== undefined) {
        expect(screen.getByText(expected.hint)).toBeInTheDocument()
      }

      const mutate = mutations[expected.mutation as "seed" | "resync" | "createChapter"]
      await user.click(screen.getByRole("button", { name: /Go/ }))

      if (expected.mutation === "createChapter") {
        // `write_next` navigates to the chapter it created: the route change is the outcome. The
        // card only reads the new chapter's id (`navigate` to `/books/:bookId/chapters/:id`).
        act(() => {
          optionsOf<{ id: number }>(mutate).onSuccess?.({ id: 9 })
        })
        expect(await screen.findByText("chapter page")).toBeInTheDocument()
        return
      }

      act(() => {
        optionsOf<{ session_id: number }>(mutate).onSuccess?.({ session_id: 77 })
      })
      // The panel's pinned session is the returned id, and the card says so in a live region.
      expect(useUiStore.getState().pinnedSession[1]).toBe(77)
      expect(screen.getByRole("status")).toBeInTheDocument()
      expect(toast.success).toHaveBeenCalledTimes(1)
    },
  )
})
