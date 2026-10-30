/** The GoalNodeDialog fields and the open-children confirmation (spec/11 §5.5). */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, describe, expect, it, vi } from "vitest"

import enCanon from "@/i18n/locales/en/canon.json"
import { ApiError } from "@/api/client"
import type { GoalNodeDto } from "@/api/gen/GoalNodeDto"

const createMutate = vi.hoisted(() => vi.fn())
const patchMutate = vi.hoisted(() => vi.fn())
const foreshadowingQuery = vi.hoisted(() => vi.fn())

vi.mock("@/api/hooks/goals", () => ({
  useCreateGoal: () => ({ mutate: createMutate, isPending: false }),
  usePatchGoal: () => ({ mutate: patchMutate, isPending: false }),
  useDeleteGoal: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/api/hooks/useCanon", () => ({
  useEntities: () => ({ data: [] }),
  useForeshadowing: (bookId: number, status?: string, trackId?: number) => {
    foreshadowingQuery(bookId, status, trackId)
    return { data: [{ id: 9, title: "Private hook", status: "open" }] }
  },
  useTimeline: () => ({ data: [] }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import { GoalNodeDialog } from "@/features/canon/GoalNodeDialog"

const node: GoalNodeDto = {
  id: 9,
  book_id: 1,
  track_id: null,
  level: "plan",
  parent_id: null,
  title: "Plan",
  purpose: "Purpose",
  status: "active",
  chapter_from: null,
  chapter_to: null,
  conditions: [],
  entities: [],
  events: [],
  foreshadowing: [],
  ready: false,
  closable: false,
  created_at: "",
  updated_at: "",
}

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

describe("GoalNodeDialog", () => {
  it("refuses to save without a purpose", async () => {
    const user = userEvent.setup()
    render(<GoalNodeDialog bookId={1} nodes={[]} open onOpenChange={vi.fn()} />)
    await user.type(screen.getByLabelText("Title"), "A goal")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(createMutate).not.toHaveBeenCalled()
  })

  it("creates a node with a purpose", async () => {
    const user = userEvent.setup()
    render(<GoalNodeDialog bookId={1} nodes={[]} open onOpenChange={vi.fn()} />)
    await user.type(screen.getByLabelText("Title"), "A goal")
    await user.type(screen.getByLabelText("Purpose"), "Why it exists")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(createMutate).toHaveBeenCalledWith(
      expect.objectContaining({ title: "A goal", purpose: "Why it exists", level: "goal" }),
      expect.anything(),
    )
  })

  it("turns goal.open_children into a confirmation and resends with the flag", async () => {
    const user = userEvent.setup()
    patchMutate.mockImplementation((_variables: unknown, options: { onError?: (error: unknown) => void }) => {
      options.onError?.(
        new ApiError(409, "goal.open_children", "open children", {
          children: [{ id: 10, level: "task", title: "Open task", status: "waiting" }],
        }),
      )
    })
    render(<GoalNodeDialog bookId={1} nodes={[node]} open onOpenChange={vi.fn()} node={node} />)
    await user.selectOptions(screen.getByLabelText("Status"), "done")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(screen.getByText("Open task")).toBeInTheDocument()
    expect(screen.getByText("Close this node anyway?")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Close anyway" }))
    expect(patchMutate.mock.calls[1][0]).toEqual(
      expect.objectContaining({
        patch: expect.objectContaining({ status: "done", confirm_open_children: true }),
      }),
    )
  })
})

describe("goal dialog scope and links", () => {
  it("takes a child's scope from its parent instead of the page track filter", async () => {
    createMutate.mockClear()
    const user = userEvent.setup()
    render(
      <GoalNodeDialog
        bookId={1}
        nodes={[node]}
        open
        onOpenChange={vi.fn()}
        parentId={node.id}
        trackId={5}
      />,
    )
    await user.type(screen.getByLabelText("Title"), "Child")
    await user.type(screen.getByLabelText("Purpose"), "Child purpose")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(createMutate.mock.calls[0][0]).toEqual(
      expect.objectContaining({ parent_id: node.id, track_id: null, level: "task" }),
    )
  })

  it("hides the change note when creating and shows names with remove controls when editing", () => {
    const view = render(<GoalNodeDialog bookId={1} nodes={[]} open onOpenChange={vi.fn()} />)
    expect(screen.queryByLabelText("Change note")).toBeNull()
    view.unmount()

    const linked = {
      ...node,
      entities: [{ id: 7, name: "Jade Pendant" }],
      events: [{ id: 8, title: "The gate" }],
      foreshadowing: [{ id: 9, title: "Broken hilt", status: "open" as const }],
    }
    render(<GoalNodeDialog bookId={1} nodes={[linked]} open onOpenChange={vi.fn()} node={linked} />)
    expect(screen.getByText("Jade Pendant")).toBeInTheDocument()
    expect(screen.getByText("The gate")).toBeInTheDocument()
    expect(screen.getByText("Broken hilt")).toBeInTheDocument()
    expect(screen.getAllByRole("button", { name: "Remove link" })).toHaveLength(3)
  })
})


describe("foreshadowing condition scope", () => {
  it("passes the private node's scope to the condition picker", async () => {
    foreshadowingQuery.mockClear()
    const privateNode: GoalNodeDto = { ...node, id: 12, track_id: 2, level: "task" }
    const user = userEvent.setup()
    render(<GoalNodeDialog bookId={1} nodes={[privateNode]} open onOpenChange={vi.fn()} node={privateNode} />)
    const before = foreshadowingQuery.mock.calls.length
    await user.click(screen.getByRole("button", { name: "Add condition" }))
    await user.selectOptions(screen.getByLabelText("Condition kind"), "foreshadowing_resolved")
    expect(foreshadowingQuery.mock.calls.length).toBeGreaterThan(before)
    expect(foreshadowingQuery.mock.calls.some((call) => call[2] === 2)).toBe(true)
    expect(screen.getAllByRole("option", { name: "Private hook" }).length).toBeGreaterThan(0)
  })
})
