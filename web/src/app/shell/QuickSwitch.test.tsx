/**
 * The quick switch (PROJECT §8.13, spec/11 §7): ⌘/Ctrl+K opens a searchable list of pages, books
 * and — inside a book — its chapters.
 *
 * This suite exists because the feature shipped completely broken and nothing caught it: the
 * shadcn `CommandDialog` wrapper put its children straight into a Radix `DialogContent` and never
 * rendered the cmdk `<Command>` root, so every `CommandInput`/`CommandItem` read `undefined` from
 * cmdk's context and threw while rendering:
 *
 *     TypeError: Cannot read properties of undefined (reading 'subscribe')
 *
 * Every leaf component of cmdk reads the state context created by `<Command>` through
 * `useSyncExternalStore(ctx.subscribe, …)`, so mounting the palette is the only way to prove the
 * root is there. A render test is exactly the coverage that was missing.
 *
 * ## jsdom gaps this suite fills locally (do not re-discover them)
 * Mounting a cmdk list inside a Radix dialog needs three things jsdom does not provide. Only the
 * third is already global, in `src/test/setup.ts`:
 * - `Element.prototype.scrollIntoView` — cmdk scrolls the selected item into view in a layout
 *   effect on every state flush (`cmdk/dist/index.mjs`, `ne()`); jsdom has no layout engine so the
 *   method does not exist and the mount throws `TypeError: i.scrollIntoView is not a function`.
 * - `window.matchMedia` — `ThemeProvider` (mounted by `AppShell` via `ThemeToggle`) reads it for
 *   the system theme; jsdom does not implement it.
 * - `ResizeObserver` — needed by cmdk's `CommandList`; **already stubbed globally**, reused here.
 * The first two are stubbed in `beforeAll`/the mount helper below, deliberately **not** in
 * `src/test/setup.ts`: that file is shared by every suite and changing it can alter other agents'
 * tests. Promote them there once the tree is quiet and a second suite needs them.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { useEffect } from "react"
import { I18nextProvider } from "react-i18next"
import { MemoryRouter, Route, Routes, useLocation } from "react-router"
import { beforeAll, describe, expect, it, vi } from "vitest"

import type { BookSummaryDto } from "@/api/gen/BookSummaryDto"
import type { ChapterListItemDto } from "@/api/gen/ChapterListItemDto"
import type { TrackDto } from "@/api/gen/TrackDto"
import { AppShell } from "@/app/shell/AppShell"
import { ThemeProvider } from "@/components/theme-provider"
import enBook from "@/i18n/locales/en/book.json"
import enCanon from "@/i18n/locales/en/canon.json"
import enKnowledge from "@/i18n/locales/en/knowledge.json"
import enPrompts from "@/i18n/locales/en/prompts.json"
import enSettings from "@/i18n/locales/en/settings.json"

import { QuickSwitch, bookIdOf } from "./QuickSwitch"

/**
 * Two books, one still in conception. `useBooks()` is called with **no status filter** — the server
 * filters exactly (`?status=conceiving|active|archived`, `books::list`), and the library bug D-129 /
 * 921ef14 was that asking for `status=active` made a conceiving book unreachable. The quick switch
 * is a jump-to list and inherits the same rule: hiding a book the author just created would repeat
 * that bug in the one place whose entire job is reaching things. The fixture pins both statuses.
 */
const BOOKS: BookSummaryDto[] = [
  {
    id: 3,
    title: "Manual Check Book",
    genre: "xianxia",
    writing_language: "zh-CN",
    status: "active",
    chapter_count: 1,
    official_chapter_count: 0,
    word_count: 8,
    latest_chapter: null,
    attention_count: 0,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  },
  {
    id: 5,
    title: "Bringing the Blue Fish",
    genre: "xianxia",
    writing_language: "zh-CN",
    status: "conceiving",
    chapter_count: 0,
    official_chapter_count: 0,
    word_count: 0,
    latest_chapter: null,
    attention_count: 0,
    created_at: "2026-01-02T00:00:00Z",
    updated_at: "2026-01-02T00:00:00Z",
  },
]

const TRACKS: TrackDto[] = [
  {
    id: 10,
    book_id: 3,
    kind: "main",
    title: "",
    synopsis: "",
    canon_mode: "canon",
    anchor_kind: "none",
    anchor_chapter_id: null,
    focus_entity_id: null,
    ord: 0,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  },
]

const CHAPTERS: ChapterListItemDto[] = [
  {
    id: 42,
    book_id: 3,
    track_id: 10,
    number: 1,
    title: "",
    status: "drafting",
    canon_status: "none",
    word_count: 8,
    updated_at: "2026-01-01T00:00:00Z",
  },
]

// The palette reads the same query keys the screens use, so the data source is stubbed here and
// the component under test keeps its own wiring (`useQueries`, `bookIdOf`, navigation).
vi.mock("@/api/hooks/books", () => ({
  useBooks: () => ({ data: BOOKS, isPending: false, isError: false }),
}))

vi.mock("@/api/hooks/tracks", () => ({
  useTracks: (bookId: number) => ({ data: bookId > 0 ? TRACKS : [], isPending: false }),
}))

vi.mock("@/api/endpoints/tracks", () => ({
  listChapters: (trackId: number) => Promise.resolve(trackId === 10 ? CHAPTERS : []),
}))

beforeAll(() => {
  // cmdk scrolls its selected item into view after every state flush; jsdom implements no layout,
  // so `Element.prototype.scrollIntoView` does not exist there. The `ResizeObserver` that cmdk's
  // `CommandList` also needs is already stubbed globally in `src/test/setup.ts`. This stub is
  // local because the quick switch is the only suite that mounts a cmdk list.
  if (!("scrollIntoView" in Element.prototype)) {
    Object.defineProperty(Element.prototype, "scrollIntoView", {
      configurable: true,
      writable: true,
      value: () => {},
    })
  }

  // The global test setup ships only a subset of the namespaces. The palette's page list spans
  // `book:`, `canon:`, `knowledge:` and `common:`, so a missing bundle would render bare keys and
  // hide a real regression behind an untranslated label.
  const extras: [string, Record<string, unknown>][] = [
    ["book", enBook],
    ["canon", enCanon],
    ["knowledge", enKnowledge],
    ["prompts", enPrompts],
    ["settings", enSettings],
  ]
  for (const [namespace, bundle] of extras) {
    if (!i18n.hasResourceBundle("en", namespace)) {
      i18n.addResourceBundle("en", namespace, bundle, true, true)
    }
    if (!i18n.options.ns?.includes(namespace)) {
      i18n.options.ns = [...(i18n.options.ns as string[]), namespace]
    }
  }
})

/** Records the router path so a selection can be checked against the route it opened. */
let currentPath = ""

function LocationProbe() {
  const location = useLocation()
  useEffect(() => {
    currentPath = location.pathname
  }, [location.pathname])
  return null
}

type MountOptions = {
  path?: string
  open?: boolean
  onOpenChange?: (open: boolean) => void
}

/** Mounts the palette the way `AppShell` does: providers plus a router at `path`. */
function mountQuickSwitch({
  path = "/",
  open = true,
  onOpenChange = () => {},
}: MountOptions = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  currentPath = path
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={[path]}>
          <LocationProbe />
          <QuickSwitch open={open} onOpenChange={onOpenChange} />
        </MemoryRouter>
      </QueryClientProvider>
    </I18nextProvider>,
  )
}

/**
 * Mounts the real `AppShell` (`AppShell.tsx`) — the header button, the lazy page outlet and the
 * window-level shortcut — so the shortcut is tested where it actually lives.
 *
 * `ThemeProvider` is needed because the shell's `ThemeToggle` reads it, and it reads
 * `window.matchMedia`, which jsdom does not implement; the stub below is the standard stand-in for
 * a media query that never matches (light theme, no reduced motion).
 */
function mountAppShell(path = "/") {
  // `"matchMedia" in window` is true under jsdom even though the value is undefined, so the check
  // has to be on the type.
  if (typeof window.matchMedia !== "function") {
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      writable: true,
      value: (query: string) => ({
        matches: false,
        media: query,
        onchange: null,
        addEventListener: () => {},
        removeEventListener: () => {},
        addListener: () => {},
        removeListener: () => {},
        dispatchEvent: () => false,
      }),
    })
  }
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  currentPath = path
  render(
    <I18nextProvider i18n={i18n}>
      <ThemeProvider>
        <QueryClientProvider client={client}>
          <MemoryRouter initialEntries={[path]}>
            <LocationProbe />
            <Routes>
              <Route path="/" element={<AppShell />}>
                <Route index element={null} />
              </Route>
            </Routes>
          </MemoryRouter>
        </QueryClientProvider>
      </ThemeProvider>
    </I18nextProvider>,
  )
}

describe("QuickSwitch", () => {
  it("renders the search input, the pages group and a book row without throwing", async () => {
    // Before the fix this render threw while mounting `CommandInput`: cmdk's leaf components read
    // the context that only the missing `<Command>` root provides, and `ctx.subscribe` was read
    // off `undefined`.
    mountQuickSwitch({ path: "/" })

    expect(await screen.findByRole("combobox")).toHaveAttribute(
      "placeholder",
      i18n.t("common:quick_switch_hint"),
    )
    expect(screen.getByRole("dialog")).toBeInTheDocument()
    expect(screen.getByText(i18n.t("common:quick_switch_pages"))).toBeInTheDocument()
    expect(screen.getByText("Manual Check Book")).toBeInTheDocument()
    expect(screen.getByText(i18n.t("common:quick_switch_books"))).toBeInTheDocument()
  })

  it("names and describes the dialog from the locale, not a hardcoded English literal", async () => {
    // Radix requires an accessible name; `aria-describedby` is announced verbatim by a screen
    // reader, so both strings have to come from the locale (AGENTS.md §6). The wrapper's English
    // defaults ("Command Palette", "Search for a command to run...") leaked into a zh-CN UI.
    mountQuickSwitch({ path: "/" })

    const dialog = await screen.findByRole("dialog", { name: i18n.t("common:quick_switch") })
    const describedBy = dialog.getAttribute("aria-describedby")
    expect(describedBy, "the dialog must be described").not.toBeNull()
    const description = document.getElementById(describedBy as string)
    expect(description).toHaveTextContent(i18n.t("common:quick_switch_hint"))
    expect(description).not.toHaveTextContent("Search for a command to run")
  })

  it("reaches a book that is still in conception (D-129, 921ef14)", async () => {
    // `useBooks()` carries no status filter, so the server returns conceiving books too. The
    // palette must list them: it is a jump-to list, and hiding a book the author just created is
    // exactly the reachability bug the library fix addressed.
    mountQuickSwitch({ path: "/" })

    expect(await screen.findByRole("option", { name: "Bringing the Blue Fish" })).toBeInTheDocument()
  })

  it("lists the open book's chapters as well", async () => {
    mountQuickSwitch({ path: "/books/3" })

    // The chapter list arrives through `useQueries`, so this also proves the list renders after
    // the query settles rather than only on the first synchronous pass.
    expect(await screen.findByText("Chapter 1")).toBeInTheDocument()
    expect(screen.getByText(i18n.t("common:quick_switch_chapters"))).toBeInTheDocument()
    // Inside a book the book's own pages are offered too (spec/11 §7).
    expect(screen.getByText(i18n.t("book:overview"))).toBeInTheDocument()
  })

  it("navigates to the selected destination and closes the dialog", async () => {
    const user = userEvent.setup()
    const onOpenChange = vi.fn()
    mountQuickSwitch({ path: "/", onOpenChange })

    const row = await screen.findByRole("option", { name: "Manual Check Book" })
    await user.click(row)

    expect(onOpenChange).toHaveBeenCalledWith(false)
    expect(currentPath).toBe("/books/3")
  })

  it("navigates to a chapter of the open book", async () => {
    const user = userEvent.setup()
    const onOpenChange = vi.fn()
    mountQuickSwitch({ path: "/books/3", onOpenChange })

    await user.click(await screen.findByRole("option", { name: "Chapter 1" }))

    expect(onOpenChange).toHaveBeenCalledWith(false)
    expect(currentPath).toBe("/books/3/chapters/42")
  })
})

describe("bookIdOf", () => {
  it("reads the book id out of a route", () => {
    const table: [string, number | null][] = [
      ["/", null],
      ["/books/12", 12],
      ["/books/12/", 12],
      ["/books/12/chapters/3", 12],
      ["/books/x", null],
      ["/books/0", null],
      ["/knowledge", null],
      ["/books", null],
    ]
    for (const [pathname, expected] of table) {
      expect(bookIdOf(pathname), pathname).toBe(expected)
    }
  })
})

/**
 * A keydown with an IME composition flag. jsdom ignores `isComposing` in the init dictionary, so it
 * has to be defined on the event itself — the same reason `lib/hotkeys.test.tsx` does it this way.
 */
function keydown(init: KeyboardEventInit & { composing?: boolean }): KeyboardEvent {
  const { composing, ...rest } = init
  const event = new KeyboardEvent("keydown", { ...rest, cancelable: true })
  if (composing === true) {
    Object.defineProperty(event, "isComposing", { value: true })
  }
  return event
}

describe("the ⌘K / Ctrl+K shortcut in the app shell", () => {
  it("opens the quick switch and closes it again", async () => {
    // Mounted through `AppShell`, which is where the shortcut lives (spec/11 §7, PROJECT §8.13):
    // a passing `useHotkey` unit test did not prove the palette it toggles can even render.
    mountAppShell()

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    fireEvent(window, keydown({ key: "k", ctrlKey: true }))
    expect(await screen.findByRole("combobox")).toBeInTheDocument()

    fireEvent(window, keydown({ key: "k", metaKey: true }))
    await waitFor(() => {
      expect(screen.queryByRole("combobox")).not.toBeInTheDocument()
    })
  })

  it("ignores the shortcut while an IME composition is active", async () => {
    // Enter and letters belong to the input method during a composition, so a Chinese input method
    // must never open the palette (AGENTS.md §6).
    mountAppShell()

    fireEvent(window, keydown({ key: "k", ctrlKey: true, composing: true }))
    // The toggle is synchronous; a microtask flush is enough to make a spurious open observable.
    await act(async () => {
      await Promise.resolve()
    })
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument()
  })
})
