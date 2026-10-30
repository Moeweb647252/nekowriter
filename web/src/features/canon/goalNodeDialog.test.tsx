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

vi.mock("@/api/hooks/goals", () => ({
  useCreateGoal: () => ({ mutate: createMutate, isPending: false }),
  usePatchGoal: () => ({ mutate: patchMutate, isPending: false }),
  useDeleteGoal: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/api/hooks/useCanon", () => ({
  useEntities: () => ({ data: [] }),
  useForeshadowing: () => ({ data: [] }),
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
