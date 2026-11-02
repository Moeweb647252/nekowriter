/**
 * The navigator toggle below 1024px (spec/11 §5.1).
 *
 * It used to be an absolute overlay inside the main panel, so on the conception route it covered
 * the studio title and it also appeared while the navigator was already expanded. These tests pin
 * the three properties the fix introduces: no toggle when expanded, an in-flow toggle when
 * collapsed, and a strip that disappears at the 1024px breakpoint.
 */
import { render, screen } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

const state = vi.hoisted(() => ({ pathname: "/books/1/conception", navCollapsed: false }))
const expand = vi.hoisted(() => vi.fn())

vi.mock("react-router", () => ({
  useParams: () => ({ bookId: "1" }),
  useNavigate: () => vi.fn(),
  useLocation: () => ({ pathname: state.pathname }),
  Outlet: () => <div data-testid="outlet" />,
}))

vi.mock("react-resizable-panels", () => ({
  useDefaultLayout: () => ({ defaultLayout: {}, onLayoutChanged: vi.fn() }),
  usePanelRef: () => ({ current: { expand } }),
}))

// The real panels need layout measurements JSDOM does not provide. This stand-in reports the nav
// panel's persisted size exactly like the library does (0 px is collapsed), which is the state the
// toggle is derived from.
vi.mock("@/components/ui/resizable", async () => {
  const React = await import("react")
  return {
    ResizablePanelGroup: ({ children }: { children: React.ReactNode }) => (
      <div>{children}</div>
    ),
    ResizableHandle: () => null,
    ResizablePanel: ({
      id,
      onResize,
      children,
    }: {
      id: string
      onResize?: (size: { inPixels: number }) => void
      children: React.ReactNode
    }) => {
      React.useEffect(() => {
        if (id === "nav") {
          onResize?.({ inPixels: state.navCollapsed ? 0 : 240 })
        }
      }, [id, onResize])
      return <div id={id}>{children}</div>
    },
  }
})

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
        canon_systems: {
          entities: true,
          timeline: true,
          memory: true,
          foreshadowing: true,
          goals: true,
        },
      },
    },
    isPending: false,
    isError: false,
  }),
  usePatchBook: () => ({ mutate: vi.fn(), isPending: false }),
}))

import BookLayout from "@/features/workspace/BookLayout"

function renderLayout() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <BookLayout />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  state.pathname = "/books/1/conception"
  state.navCollapsed = false
  expand.mockReset()
})

describe("navigator toggle", () => {
  it("shows no toggle while the navigator is expanded", () => {
    state.navCollapsed = false
    renderLayout()
    expect(screen.queryByRole("button", { name: "Toggle navigator" })).toBeNull()
    expect(screen.queryByTestId("navigator-toggle-strip")).toBeNull()
  })

  it("shows the toggle in flow, not absolutely positioned, while the navigator is collapsed", () => {
    state.navCollapsed = true
    renderLayout()
    const button = screen.getByRole("button", { name: "Toggle navigator" })
    const strip = screen.getByTestId("navigator-toggle-strip")
    expect(strip).toContainElement(button)
    expect(strip.className).toContain("flex")
    // The strip reserves its own row below 1024px and is display-none above it.
    expect(strip.className).toContain("[@media(min-width:1024px)]:hidden")
    expect(strip.className).not.toContain("absolute")
    expect(button.className).not.toContain("absolute")
  })

  it("expands the navigator from the in-flow toggle", async () => {
    state.navCollapsed = true
    renderLayout()
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Toggle navigator" }))
    expect(expand).toHaveBeenCalledOnce()
  })
})
