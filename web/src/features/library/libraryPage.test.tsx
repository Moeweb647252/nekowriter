/**
 * The library shows every book the author can still work on (PROJECT §8.3, spec/11 §3, D-129).
 *
 * The bug this pins: the page asked the server for `status=active` whenever "Show archived" was
 * off, and `books::list` filters server-side, so a book in conception — created with "start with
 * the conception agent" and not yet finalized — never arrived and no client-side filter could
 * recover it. The default view therefore dropped exactly the book the author was working on, and
 * a conceiving book stayed unreachable from the library from creation until finalize.
 *
 * The fix asks the server for every status once and hides archived rows in the browser; the
 * conceiving card carries a status badge and links into its studio.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { MemoryRouter, Route, Routes } from "react-router"
import { beforeEach, describe, expect, it, vi } from "vitest"

import type { BookSummaryDto } from "@/api/gen/BookSummaryDto"

const booksArgs = vi.fn()
const books = vi.hoisted(() => ({ value: [] as BookSummaryDto[] }))

vi.mock("@/api/hooks/books", () => ({
  useBooks: (status?: string) => {
    booksArgs(status)
    // Mirrors `GET /api/books?status=`: the server filters, so a status the page did not ask for
    // never reaches the browser. Emulating that here is what makes the conceiving card disappear
    // for the real reason instead of only for a mocked-away query.
    const rows =
      status === undefined
        ? books.value
        : books.value.filter((book) => book.status === status)
    return { data: rows, isPending: false, isError: false, refetch: vi.fn() }
  },
  useCreateBook: () => ({ mutate: vi.fn(), isPending: false }),
  usePatchBook: () => ({ mutate: vi.fn(), isPending: false }),
  useDeleteBook: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import LibraryPage from "@/features/library/LibraryPage"

function book(overrides: Partial<BookSummaryDto>): BookSummaryDto {
  return {
    id: 1,
    title: "Manual Check Book",
    genre: "xianxia",
    writing_language: "zh-CN",
    status: "active",
    chapter_count: 0,
    official_chapter_count: 0,
    word_count: 0,
    latest_chapter: null,
    attention_count: 0,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  }
}

const ACTIVE = book({ id: 1, title: "Manual Check Book", status: "active" })
const CONCEIVING = book({
  id: 3,
  title: "带着蓝色大肥鱼穿洪武",
  status: "conceiving",
})
const ARCHIVED = book({ id: 4, title: "Old Draft", status: "archived" })

/** The library plus stand-ins for the two workspaces a card can open. */
function renderLibrary() {
  return render(
    <MemoryRouter initialEntries={["/"]}>
      <Routes>
        <Route path="/" element={<LibraryPage />} />
        <Route path="/books/:bookId" element={<p>the overview</p>} />
        <Route path="/books/:bookId/conception" element={<p>the conception studio</p>} />
      </Routes>
    </MemoryRouter>,
  )
}

describe("LibraryPage", () => {
  beforeEach(() => {
    booksArgs.mockClear()
    books.value = [ACTIVE, CONCEIVING, ARCHIVED]
  })

  it("renders a conceiving book with Show archived off", () => {
    renderLibrary()
    // The regression this test exists for: the page must not ask the server for active books
    // only, because that filter is what hid the conceiving book before it reached the browser.
    expect(booksArgs).not.toHaveBeenCalledWith("active")
    expect(screen.getByText("带着蓝色大肥鱼穿洪武")).toBeInTheDocument()
    expect(screen.getByText("In conception")).toBeInTheDocument()
  })

  it("shows a conceiving book's real chapter counts instead of hiding them", () => {
    // Real data from the author's database: book 3 is `conceiving` and already owns a chapter.
    // Replacing the counts with the "continue conception" block would hide that.
    books.value = [
      book({
        id: 3,
        title: "带着蓝色大肥鱼穿洪武",
        status: "conceiving",
        chapter_count: 1,
        word_count: 0,
        latest_chapter: { id: 2, number: 1, title: "", status: "planned", canon_status: "none" },
      }),
    ]
    renderLibrary()
    expect(screen.getByText("1 ch · 0 words")).toBeInTheDocument()
    expect(screen.getByText("1. Untitled chapter")).toBeInTheDocument()
    expect(screen.getByRole("link", { name: "Continue conception" })).toBeInTheDocument()
  })

  it("keeps archived books hidden until Show archived is on", async () => {
    const user = userEvent.setup()
    renderLibrary()
    expect(screen.queryByText("Old Draft")).not.toBeInTheDocument()

    await user.click(screen.getByLabelText("Show archived"))
    expect(screen.getByText("Old Draft")).toBeInTheDocument()
    // The conceiving book stays visible either way: it is not archived.
    expect(screen.getByText("带着蓝色大肥鱼穿洪武")).toBeInTheDocument()
  })

  it("takes a conceiving book straight into its studio", async () => {
    const user = userEvent.setup()
    renderLibrary()
    expect(screen.getByRole("link", { name: "Continue conception" })).toHaveAttribute(
      "href",
      "/books/3/conception",
    )
    await user.click(screen.getByRole("button", { name: "带着蓝色大肥鱼穿洪武" }))
    expect(screen.getByText("the conception studio")).toBeInTheDocument()
  })

  it("opens an active book's overview when its title is clicked", async () => {
    books.value = [ACTIVE]
    const user = userEvent.setup()
    renderLibrary()
    expect(screen.queryByRole("link", { name: "Continue conception" })).not.toBeInTheDocument()
    expect(screen.queryByText("In conception")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Manual Check Book" }))
    expect(screen.getByText("the overview")).toBeInTheDocument()
  })
})
