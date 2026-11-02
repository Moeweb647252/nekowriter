/** The full entity history toggle (spec/06 §3, spec/11 §5.5, D-161). */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import enCanon from "@/i18n/locales/en/canon.json"
import type { EntityDto } from "@/api/gen/EntityDto"

const historyCalls = vi.hoisted(() => [] as (number | undefined)[])

vi.mock("react-router", () => ({
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => (
    <a href={to}>{children}</a>
  ),
}))
vi.mock("@/api/hooks/useCanon", () => ({
  useEntityHistory: (entityId: number | undefined) => {
    historyCalls.push(entityId)
    if (entityId === undefined) {
      return { data: undefined, isPending: false }
    }
    return {
      data: Array.from({ length: entityId === 1 ? 100 : 40 }, (_, index) => ({
        id: index + 1,
      })),
      isPending: false,
    }
  },
  useEntities: () => ({ data: [] }),
  useEntity: () => ({ data: undefined }),
  usePatchEntity: () => ({ mutate: vi.fn(), isPending: false }),
  usePatchRelation: () => ({ mutate: vi.fn(), isPending: false }),
  useCreateRelation: () => ({ mutate: vi.fn(), isPending: false }),
  useDeleteRelation: () => ({ mutate: vi.fn(), isPending: false }),
  useDeleteEntity: () => ({ mutate: vi.fn(), isPending: false }),
  useSetProtagonist: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))
vi.mock("@/features/canon/ChangeList", () => ({
  ChangeList: ({ changes }: { changes: unknown[] }) => (
    <div data-testid="change-list">{changes.length}</div>
  ),
}))

import { HistorySection } from "@/features/canon/EntityDetail"

function changes(count: number): EntityDto["recent_changes"] {
  return Array.from({ length: count }, (_, index) => ({ id: index + 1 })) as unknown as EntityDto["recent_changes"]
}

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

beforeEach(() => {
  historyCalls.length = 0
})

describe("HistorySection", () => {
  it("offers the full history only when the 20-row preview is truncated", () => {
    const { rerender } = render(
      <HistorySection bookId={1} entityId={1} recentChanges={changes(19)} />,
    )
    expect(screen.queryByRole("button", { name: "Show full history" })).toBeNull()

    rerender(<HistorySection bookId={1} entityId={1} recentChanges={changes(20)} />)
    expect(screen.getByRole("button", { name: "Show full history" })).toBeInTheDocument()
  })

  it("loads the history and states the 100-row cap", async () => {
    render(<HistorySection bookId={1} entityId={1} recentChanges={changes(20)} />)
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Show full history" }))

    expect(historyCalls).toContain(1)
    expect(screen.getByTestId("change-list")).toHaveTextContent("100")
    expect(
      screen.getByText("Only the latest 100 changes are shown."),
    ).toBeInTheDocument()
    expect(screen.getByRole("link", { name: "Open the changes page" })).toHaveAttribute(
      "href",
      "/books/1/canon/changes",
    )
  })

  it("resets the expanded history when the entity changes", async () => {
    const { rerender } = render(
      <HistorySection bookId={1} entityId={1} recentChanges={changes(20)} />,
    )
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Show full history" }))
    expect(screen.getByTestId("change-list")).toHaveTextContent("100")

    rerender(<HistorySection bookId={1} entityId={2} recentChanges={changes(20)} />)
    expect(screen.getByTestId("change-list")).toHaveTextContent("20")
    expect(screen.getByRole("button", { name: "Show full history" })).toBeInTheDocument()
  })
})
