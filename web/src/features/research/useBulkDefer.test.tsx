/**
 * The bulk defer operation (D-139): every `ready` task of one book, one request at a time.
 *
 * The author reported eight separate clicks to postpone eight tasks, and the overview card that
 * demanded the research offered no way to say "not now" from where the demand appears. The batch
 * lives in one place (`useDeferReadyResearchTasks`) so the list and the card cannot drift in
 * scoping, sequencing, reporting or invalidation; these tests pin that place directly, against the
 * real TanStack Query cache and the real `useDeferResearchTask` invalidation.
 */
import type { ReactNode } from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { ResearchTaskDto } from "@/api/gen/ResearchTaskDto"

const api = vi.hoisted(() => ({
  list: vi.fn(),
  defer: vi.fn(),
}))

vi.mock("@/api/endpoints/research", () => ({
  listResearchTasks: api.list,
  deferResearchTask: api.defer,
  resumeResearchTask: vi.fn(),
  getResearchTask: vi.fn(),
  createResearchTask: vi.fn(),
  patchResearchTask: vi.fn(),
  deleteResearchTask: vi.fn(),
  runResearchTask: vi.fn(),
}))

const toasts = vi.hoisted(() => ({
  success: vi.fn(),
  warning: vi.fn(),
  error: vi.fn(),
}))

vi.mock("sonner", () => ({ toast: toasts }))

import i18n from "@/i18n"
import enKnowledge from "@/i18n/locales/en/knowledge.json"
import zhKnowledge from "@/i18n/locales/zh-CN/knowledge.json"
import { keys } from "@/api/keys"
import { useDeferReadyResearchTasks } from "@/features/research/useBulkDefer"

beforeAll(() => {
  // The setup file loads English only; the toast assertions below are English, and the zh-CN
  // bundle is loaded so the parity test can read both (spec/11 §6).
  for (const [language, bundle] of [
    ["en", enKnowledge],
    ["zh-CN", zhKnowledge],
  ] as const) {
    i18n.addResourceBundle(language, "knowledge", bundle, true, true)
  }
  const namespaces = (i18n.options.ns ?? []) as string[]
  if (!namespaces.includes("knowledge")) {
    i18n.options.ns = [...namespaces, "knowledge"]
  }
})

/** A research task row; only the fields the batch reads are meaningful. */
function task(overrides: Partial<ResearchTaskDto> = {}): ResearchTaskDto {
  return {
    id: 1,
    book_id: 3,
    scope_id: 1,
    scope_name: "明初历史",
    topic: "宝钞与财政",
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

function wrapperFor(client: QueryClient) {
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
}

function newClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } })
}

/** True once the query for `queryKey` has been invalidated in the cache. */
function isInvalidated(client: QueryClient, queryKey: readonly unknown[]): boolean {
  return client.getQueryCache().find({ queryKey })?.state.isInvalidated ?? false
}

beforeEach(() => {
  api.list.mockReset()
  api.defer.mockReset()
  toasts.success.mockReset()
  toasts.warning.mockReset()
  toasts.error.mockReset()
  // The default: a successful defer answers the row it moved (spec/10 §2.5).
  api.defer.mockImplementation(async (taskId: number) => task({ id: taskId, status: "deferred" }))
})

describe("useDeferReadyResearchTasks", () => {
  it("defers exactly the book's ready tasks, one request at a time", async () => {
    const rows = [
      task({ id: 1, status: "ready" }),
      task({ id: 2, status: "deferred" }),
      task({ id: 3, status: "ready" }),
      task({ id: 4, status: "draft" }),
      task({ id: 5, status: "done" }),
      task({ id: 6, status: "ready" }),
    ]
    api.list.mockResolvedValue(rows)
    // Sequential, not parallel: the API is per-task and transactional, and a mid-way failure has
    // to stay attributable to one task.
    let inFlight = 0
    let peak = 0
    api.defer.mockImplementation(async (taskId: number) => {
      inFlight += 1
      peak = Math.max(peak, inFlight)
      await new Promise((resolve) => setTimeout(resolve, 0))
      inFlight -= 1
      return task({ id: taskId, status: "deferred" })
    })
    const client = newClient()
    const { result } = renderHook(() => useDeferReadyResearchTasks({ bookId: 3 }), {
      wrapper: wrapperFor(client),
    })
    await waitFor(() => expect(result.current.readyTasks).toHaveLength(3))

    // The candidates come from the same book-scoped query the list renders.
    expect(api.list).toHaveBeenCalledWith({ bookId: 3 })
    expect(result.current.readyTasks.map((row) => row.id)).toEqual([1, 3, 6])

    let outcome: Awaited<ReturnType<typeof result.current.run>> | undefined
    await act(async () => {
      outcome = await result.current.run()
    })

    // Only `ready` rows, in list order, exactly once each.
    expect(api.defer.mock.calls.map((call) => call[0])).toEqual([1, 3, 6])
    expect(peak).toBe(1)
    expect(outcome).toEqual(
      expect.objectContaining({ deferred: [1, 3, 6], failed: [] }),
    )
    expect(toasts.success).toHaveBeenCalledWith("Deferred 3 research tasks")
    expect(toasts.warning).not.toHaveBeenCalled()
  })

  it("never touches another book's task, even if the row is in the list", async () => {
    // Defence in depth: the server already filters by `book_id`, and the batch filters again so a
    // stale or wrongly shaped cache entry cannot postpone unrelated work (D-139).
    api.list.mockResolvedValue([
      task({ id: 1, book_id: 3 }),
      task({ id: 2, book_id: 9 }),
    ])
    const client = newClient()
    const { result } = renderHook(() => useDeferReadyResearchTasks({ bookId: 3 }), {
      wrapper: wrapperFor(client),
    })
    await waitFor(() => expect(result.current.readyTasks).toHaveLength(1))

    await act(async () => {
      await result.current.run()
    })

    expect(api.defer.mock.calls.map((call) => call[0])).toEqual([1])
  })

  it("reports a partial failure with how many succeeded and which tasks failed", async () => {
    api.list.mockResolvedValue([
      task({ id: 1, topic: "洪武朝大事编年" }),
      task({ id: 2, topic: "大明宝钞与明初货币财政" }),
      task({ id: 3, topic: "明代宗室制度" }),
    ])
    api.defer.mockImplementation(async (taskId: number) => {
      if (taskId === 2) {
        throw new Error("boom")
      }
      return task({ id: taskId, status: "deferred" })
    })
    const client = newClient()
    const { result } = renderHook(() => useDeferReadyResearchTasks({ bookId: 3 }), {
      wrapper: wrapperFor(client),
    })
    await waitFor(() => expect(result.current.readyTasks).toHaveLength(3))

    let outcome: Awaited<ReturnType<typeof result.current.run>> | undefined
    await act(async () => {
      outcome = await result.current.run()
    })

    // The failing task does not abort the batch: 2 and 3 are still attempted.
    expect(api.defer.mock.calls.map((call) => call[0])).toEqual([1, 2, 3])
    expect(outcome).toEqual(
      expect.objectContaining({
        deferred: [1, 3],
        failed: [
          { taskId: 2, topic: "大明宝钞与明初货币财政", error: expect.any(Error) },
        ],
      }),
    )
    // Visible, not silent: the count and the topic of the task that did not move.
    expect(toasts.warning).toHaveBeenCalledWith("Deferred 2 of 3 research tasks", {
      description: "Not deferred: 大明宝钞与明初货币财政",
    })
    expect(toasts.success).not.toHaveBeenCalled()
  })

  it("reports an all-failed batch as an error, not a warning", async () => {
    api.list.mockResolvedValue([task({ id: 1, topic: "宝钞与财政" })])
    api.defer.mockRejectedValue(new Error("offline"))
    const client = newClient()
    const { result } = renderHook(() => useDeferReadyResearchTasks({ bookId: 3 }), {
      wrapper: wrapperFor(client),
    })
    await waitFor(() => expect(result.current.readyTasks).toHaveLength(1))

    await act(async () => {
      await result.current.run()
    })

    expect(toasts.error).toHaveBeenCalledWith("Deferred 0 of 1 research task", {
      description: "Not deferred: 宝钞与财政",
    })
    expect(toasts.warning).not.toHaveBeenCalled()
  })

  it("refetches the list and that book's overview, and no other book's", async () => {
    api.list.mockResolvedValue([task({ id: 1, book_id: 3 })])
    const client = newClient()
    // The overview of this book is cached but not mounted (the author is on the list): the
    // invalidation is what makes the card refetch when they navigate back (D-138).
    client.setQueryData(keys.books.overview(3), {})
    client.setQueryData(keys.books.overview(4), {})
    const { result } = renderHook(() => useDeferReadyResearchTasks({ bookId: 3 }), {
      wrapper: wrapperFor(client),
    })
    await waitFor(() => expect(result.current.readyTasks).toHaveLength(1))
    expect(api.list).toHaveBeenCalledTimes(1)

    await act(async () => {
      await result.current.run()
    })

    // The list the author is looking at refetches, so the rows show the new statuses; the
    // overview card that demanded the research is invalidated, so it stops offering `run_research`
    // (without it the card kept asking, D-138/D-139).
    await waitFor(() => expect(api.list).toHaveBeenCalledTimes(2))
    expect(isInvalidated(client, keys.books.overview(3))).toBe(true)
    // Another book's overview is untouched: the batch is book-scoped, so its invalidation is too.
    expect(isInvalidated(client, keys.books.overview(4))).toBe(false)
  })

  it("defers across books on the global list, which is not book-scoped", async () => {
    api.list.mockResolvedValue([
      task({ id: 1, book_id: 3 }),
      task({ id: 2, book_id: 9 }),
    ])
    const client = newClient()
    const { result } = renderHook(() => useDeferReadyResearchTasks(), {
      wrapper: wrapperFor(client),
    })
    await waitFor(() => expect(result.current.readyTasks).toHaveLength(2))

    expect(api.list).toHaveBeenCalledWith({ bookId: undefined })
    await act(async () => {
      await result.current.run()
    })
    expect(api.defer.mock.calls.map((call) => call[0])).toEqual([1, 2])
  })

  it("carries every report in both locales", () => {
    // A count goes through interpolation, never through concatenated fragments (AGENTS §6).
    for (const [language, table] of [
      ["en", enKnowledge],
      ["zh-CN", zhKnowledge],
    ] as const) {
      const section = (table as unknown as { research: Record<string, unknown> }).research
      for (const key of [
        "defer_all",
        "bulk_defer_done_one",
        "bulk_defer_done_other",
        "bulk_defer_partial_one",
        "bulk_defer_partial_other",
        "bulk_defer_failed_list",
      ] as const) {
        const value = section[key]
        expect(value, `${language} knowledge:research.${key}`).toBeTypeOf("string")
        expect((value as string).length, `${language} knowledge:research.${key}`).toBeGreaterThan(0)
        expect(value, `${language} knowledge:research.${key}`).toMatch(/\{\{\w+\}\}/)
      }
    }
  })
})
