/**
 * The workspace's route-aware height chain (PROJECT §8.7, spec/11 §5.1/§5.4).
 *
 * The conception studio needs a bounded main panel, because its conversation pane and its document
 * column scroll inside it and the composer must stay put. Every other route is a page whose content
 * is taller than the viewport and which relies on this panel scrolling the whole thing — the
 * chapter page starts with `flex min-h-full flex-col`, and the canon and settings-materials pages
 * lay out their own inner scrollers. A regression here would either bring back "the composer
 * scrolls away" (studio) or break page scrolling on every other route, so both directions are
 * asserted.
 *
 * react-resizable-panels renders each panel as an outer `[data-panel]` box plus an inner wrapper
 * that carries the `className` and `style` props and is the actual `overflow` container. The
 * assertions therefore look at that inner wrapper.
 *
 * The last block covers the shell the layout hands its routed pages (D-135): the overview starts
 * canon seeding from there and has to know whether the agent panel is collapsed, and has to be able
 * to reopen it.
 */
import { render, screen } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { describe, expect, it, vi } from "vitest"

const state = vi.hoisted(() => ({
  pathname: "/books/1/conception",
  systems: { entities: true, timeline: true, memory: true, foreshadowing: true, goals: true },
}))
const patchMutate = vi.hoisted(() => vi.fn())

/**
 * The shell the layout hands its routed pages. It is captured rather than mocked: the overview's
 * canon actions reopen the agent panel through it (D-135), so what `BookLayout` provides is the
 * thing under test.
 */
const shell = vi.hoisted(() => ({ value: null as unknown }))

vi.mock("react-router", () => ({
  useParams: () => ({ bookId: "1" }),
  useNavigate: () => vi.fn(),
  useLocation: () => ({ pathname: state.pathname }),
  Outlet: (props: { context: unknown }) => {
    shell.value = props.context
    return <div data-testid="outlet" />
  },
}))

// The three panels' contents have their own tests; here only the panel structure and the wrapper
// that carries the height chain matter, so they are replaced by markers.
vi.mock("@/features/workspace/BookBreadcrumb", () => ({
  BookBreadcrumb: () => <div data-testid="breadcrumb" />,
}))

vi.mock("@/features/workspace/Navigator", () => ({
  Navigator: () => <div data-testid="navigator" />,
}))

vi.mock("@/features/agent/AgentPanel", () => ({
  AgentPanel: () => <div data-testid="agent-panel" />,
}))

vi.mock("@/api/events", () => ({
  connectEvents: vi.fn(),
  disconnectEvents: vi.fn(),
}))

vi.mock("@/api/hooks/books", () => ({
  useBook: () => ({
    data: {
      id: 1,
      title: "青云劫",
      status: "conceiving",
      settings: {
        granularity: "paragraph",
        canon_systems: state.systems,
      },
    },
    isPending: false,
    isError: false,
  }),
  usePatchBook: () => ({ mutate: patchMutate, isPending: false }),
}))

import BookLayout, { isConceptionStudio } from "@/features/workspace/BookLayout"
import type { BookShell } from "@/features/workspace/shell"

/** The app (and thus every routed page) is wrapped in a QueryClient (spec/11 §4.1). */
function renderLayout() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={client}>
      <BookLayout />
    </QueryClientProvider>,
  )
}

/**
 * The main panel's two layers: the frame `BookLayout` renders around the outlet, and the library's
 * own scroll wrapper that receives the panel's `className` and `style` props.
 */
function mainPanel(container: HTMLElement): { frame: HTMLElement; scroller: HTMLElement } {
  const frame = screen.getByTestId("outlet").parentElement
  const scroller = frame?.parentElement
  if (frame === null || scroller === null || scroller === undefined) {
    throw new Error("the outlet has no wrapper")
  }
  expect(container.contains(scroller)).toBe(true)
  // Both belong to the main panel, not to the nav panel.
  expect(scroller.closest("#main")).not.toBeNull()
  return { frame, scroller }
}

describe("BookLayout height chain", () => {
  it("recognizes the conception route and nothing else as the studio", () => {
    expect(isConceptionStudio("/books/1/conception")).toBe(true)
    // Every other route must keep page scrolling; a path that merely contains the word is not the
    // studio.
    expect(isConceptionStudio("/books/1")).toBe(false)
    expect(isConceptionStudio("/books/1/chapters/2")).toBe(false)
    expect(isConceptionStudio("/books/1/canon/entities/3")).toBe(false)
    expect(isConceptionStudio("/books/1/settings/outline")).toBe(false)
    expect(isConceptionStudio("/books/1/conception-notes")).toBe(false)
  })

  it("gives the studio a full-height wrapper and takes the panel's scrollbar away", () => {
    state.pathname = "/books/1/conception"
    const { container } = renderLayout()

    const { frame, scroller } = mainPanel(container)
    // `h-full` is the definite height the studio's own `h-full` root resolves against. It replaces
    // `min-h-full`, which would let the content grow instead of bounding it.
    expect(frame.classList.contains("h-full")).toBe(true)
    expect(frame.classList.contains("min-h-full")).toBe(false)

    // The panel itself must not scroll: the studio owns one scrollbar per pane, not a third one
    // around both (that third one is what let the composer scroll out of view).
    expect(scroller.style.overflow).toBe("hidden")
  })

  it("keeps page scrolling on every other workspace route", () => {
    for (const pathname of [
      "/books/1",
      "/books/1/chapters/2",
      "/books/1/canon/entities",
      "/books/1/settings/outline",
    ]) {
      state.pathname = pathname
      const { container, unmount } = renderLayout()

      const { frame, scroller } = mainPanel(container)
      // `min-h-full` lets the content grow past the panel, which is what page scrolling needs.
      expect(frame.classList.contains("min-h-full"), pathname).toBe(true)
      expect(frame.classList.contains("h-full"), pathname).toBe(false)
      // No inline override, so react-resizable-panels' own `overflow: auto` stands.
      expect(scroller.style.overflow, pathname).toBe("auto")

      unmount()
    }
  })

  it("keeps the agent panel on the other routes and drops it in the studio", () => {
    state.pathname = "/books/1/conception"
    const studio = renderLayout()
    expect(screen.queryByTestId("agent-panel")).toBeNull()
    studio.unmount()

    state.pathname = "/books/1/chapters/2"
    renderLayout()
    expect(screen.getByTestId("agent-panel")).toBeInTheDocument()
  })
})

describe("the workspace shell handed to routed pages (D-135)", () => {
  /** The shell the last render provided. */
  function providedShell(): BookShell {
    const value = shell.value as BookShell | null
    if (value === null) {
      throw new Error("the layout provided no shell")
    }
    return value
  }

  it("exposes the book and a way to reopen the agent panel", () => {
    state.pathname = "/books/1"
    renderLayout()

    const provided = providedShell()
    expect(provided.book.id).toBe(1)
    expect(provided.book.title).toBe("青云劫")
    // The overview calls this after starting an archivist run; it must be callable and safe even
    // when the panel is already open (a no-op, not a throw).
    expect(() => provided.expandAgentPanel()).not.toThrow()
  })

  it("reports the panel as open by default, so the seeding hint words itself for a visible panel", () => {
    state.pathname = "/books/1"
    const view = renderLayout()
    const provided = providedShell()
    // The agent panel is not collapsed by default (`defaultSize="380px"`, `collapsible` only so the
    // author can collapse it, spec/11 §5.1): if this ever becomes true by default, the overview's
    // hint would say "collapsed" on a fresh workspace, which is the case the report is about.
    expect(provided.agentPanelCollapsed).toBe(false)
    view.unmount()
  })
})


describe("disabled-system page banner", () => {
  it("shows the off banner and turns the system on through the settings save", async () => {
    state.pathname = "/books/1/canon/entities"
    state.systems = {
      entities: false,
      timeline: true,
      memory: true,
      foreshadowing: true,
      goals: true,
    }
    patchMutate.mockClear()
    renderLayout()
    expect(
      screen.getByText("This system is off for this book: agents neither read nor maintain it."),
    ).toBeInTheDocument()
    const user = await import("@testing-library/user-event").then((module) => module.default.setup())
    await user.click(screen.getByRole("button", { name: "Turn on" }))
    expect(patchMutate).toHaveBeenCalledWith(
      expect.objectContaining({
        settings: expect.objectContaining({
          canon_systems: expect.objectContaining({ entities: true }),
        }),
      }),
      expect.anything(),
    )
  })
})
