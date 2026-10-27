/**
 * "Defer all" on the research list (D-139).
 *
 * The author had eight `ready` tasks and no bulk action, so postponing them was eight clicks; the
 * overview kept insisting on research the whole time. These tests render the **real** list and the
 * **real** batch hook against a real query cache and mock only the HTTP endpoints, so what they
 * assert is the whole path: which ids the batch defers, in what order, what the author sees when
 * part of it fails, and that the rows reflect the new statuses afterwards.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { MemoryRouter } from "react-router"
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
import enChapter from "@/i18n/locales/en/chapter.json"
import enCommon from "@/i18n/locales/en/common.json"
import enKnowledge from "@/i18n/locales/en/knowledge.json"
import zhKnowledge from "@/i18n/locales/zh-CN/knowledge.json"

import { ResearchTaskList } from "./ResearchTaskList"

beforeAll(() => {
  for (const [namespace, bundle] of [
    ["knowledge", enKnowledge],
    ["chapter", enChapter],
    ["common", enCommon],
  ] as const) {
    if (!i18n.hasResourceBundle("en", namespace)) {
      i18n.addResourceBundle("en", namespace, bundle)
    }
    const namespaces = (i18n.options.ns ?? []) as string[]
    if (!namespaces.includes(namespace)) {
      i18n.options.ns = [...namespaces, namespace]
    }
  }
  // The report's zh-CN side is asserted by the parity test in `useBulkDefer.test.tsx`; this file
  // reads the English bundle only.
  void zhKnowledge
})

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

function renderList(props: { bookId?: number } = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <ResearchTaskList {...props} />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  api.list.mockReset()
  api.defer.mockReset()
  toasts.success.mockReset()
  toasts.warning.mockReset()
  toasts.error.mockReset()
  api.defer.mockImplementation(async (taskId: number) =>
    task({ id: taskId, status: "deferred" }),
  )
})

describe("deferring the whole list", () => {
  it("shows the count of runnable tasks and defers exactly those", async () => {
    const user = userEvent.setup()
    api.list.mockResolvedValue([
      task({ id: 1, status: "ready", topic: "洪武朝大事编年" }),
      task({ id: 2, status: "ready", topic: "大明宝钞" }),
      task({ id: 3, status: "deferred", topic: "明代宗室制度" }),
      task({ id: 4, status: "draft", topic: "人痘接种术" }),
      task({ id: 5, status: "done", topic: "印刷技术" }),
      task({ id: 6, status: "ready", topic: "男频连载结构" }),
    ])
    renderList({ bookId: 3 })

    // The scope of the click is visible before it happens: three of six rows are runnable.
    const button = await screen.findByRole("button", { name: "Defer all (3)" })
    await user.click(button)

    // One request per `ready` row, in list order, and nothing for deferred/draft/done.
    await waitFor(() => {
      expect(api.defer.mock.calls.map((call) => call[0])).toEqual([1, 2, 6])
    })
    // The batch is book-scoped: the same query the list renders, filtered by `book_id`.
    expect(api.list).toHaveBeenCalledWith({ bookId: 3 })
    expect(toasts.success).toHaveBeenCalledWith("Deferred 3 research tasks")
    expect(toasts.warning).not.toHaveBeenCalled()
  })

  it("defers only the book's own tasks on a book-scoped list", async () => {
    const user = userEvent.setup()
    // A stale cache entry from another book must not be postponed by this book's control.
    api.list.mockResolvedValue([
      task({ id: 1, book_id: 3, status: "ready" }),
      task({ id: 2, book_id: 9, status: "ready" }),
    ])
    renderList({ bookId: 3 })

    await user.click(await screen.findByRole("button", { name: "Defer all (1)" }))

    await waitFor(() => {
      expect(api.defer.mock.calls.map((call) => call[0])).toEqual([1])
    })
  })

  it("is absent when nothing is runnable and when the list is empty", async () => {
    api.list.mockResolvedValue([
      task({ id: 1, status: "deferred" }),
      task({ id: 2, status: "done" }),
    ])
    const populated = renderList({ bookId: 3 })
    await screen.findAllByText("宝钞与财政")
    // A disabled "Defer all (0)" would be a control that can never do anything.
    expect(screen.queryByRole("button", { name: /Defer all/ })).toBeNull()
    populated.unmount()

    api.list.mockResolvedValue([])
    renderList({ bookId: 3 })
    await screen.findByText("No research tasks yet")
    expect(screen.queryByRole("button", { name: /Defer all/ })).toBeNull()
  })

  it("reports a partial failure inline, with how many moved and which did not", async () => {
    const user = userEvent.setup()
    api.list.mockResolvedValue([
      task({ id: 1, status: "ready", topic: "洪武朝大事编年" }),
      task({ id: 2, status: "ready", topic: "大明宝钞与明初货币财政" }),
      task({ id: 3, status: "ready", topic: "明代宗室制度" }),
    ])
    api.defer.mockImplementation(async (taskId: number) => {
      if (taskId === 2) {
        throw new Error("boom")
      }
      return task({ id: taskId, status: "deferred" })
    })
    renderList({ bookId: 3 })

    await user.click(await screen.findByRole("button", { name: "Defer all (3)" }))

    // The failed task does not stop the batch: 1 and 3 moved, 2 did not.
    await waitFor(() => {
      expect(api.defer.mock.calls.map((call) => call[0])).toEqual([1, 2, 3])
    })
    // A toast is transient; the report stays on the list, which is where the author sees the
    // rows that did not change (D-139).
    const report = await screen.findByRole("alert")
    expect(within(report).getByText("Deferred 2 of 3 research tasks")).toBeInTheDocument()
    expect(
      within(report).getByText("Not deferred: 大明宝钞与明初货币财政"),
    ).toBeInTheDocument()
    expect(toasts.warning).toHaveBeenCalledWith("Deferred 2 of 3 research tasks", {
      description: "Not deferred: 大明宝钞与明初货币财政",
    })
  })

  it("refetches the rows so the moved tasks show as deferred", async () => {
    const user = userEvent.setup()
    let rows = [task({ id: 1, status: "ready" }), task({ id: 2, status: "ready" })]
    api.list.mockImplementation(async () => rows)
    api.defer.mockImplementation(async (taskId: number) => {
      rows = rows.map((row) =>
        row.id === taskId ? { ...row, status: "deferred" as const } : row,
      )
      return task({ id: taskId, status: "deferred" })
    })
    renderList({ bookId: 3 })

    await user.click(await screen.findByRole("button", { name: "Defer all (2)" }))

    // Both rows now carry the deferred badge and offer Resume; the control is gone because
    // nothing is runnable any more.
    await waitFor(() => {
      expect(screen.getAllByText("Deferred")).toHaveLength(2)
    })
    expect(screen.getAllByRole("button", { name: "Resume" })).toHaveLength(2)
    expect(screen.queryByRole("button", { name: /Defer all/ })).toBeNull()
  })
})
