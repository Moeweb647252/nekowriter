/** The hand-merge entity dialog (spec/06 §9, spec/11 §5.5, D-161). */
import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import enCanon from "@/i18n/locales/en/canon.json"
import type { EntityDto } from "@/api/gen/EntityDto"

const mergeMutate = vi.hoisted(() => vi.fn())
const navigate = vi.hoisted(() => vi.fn())
const toastSuccess = vi.hoisted(() => vi.fn())

const state = vi.hoisted(() => ({
  shared: [] as unknown[],
  sources: [] as unknown[],
  source: undefined as unknown,
}))

vi.mock("react-router", () => ({ useNavigate: () => navigate }))
vi.mock("sonner", () => ({ toast: { success: toastSuccess } }))
vi.mock("@/api/hooks/useCanon", () => ({
  useMergeEntity: () => ({ mutate: mergeMutate, isPending: false }),
  useEntities: () => ({ data: state.shared }),
  useMergeSourceEntities: () => ({ data: state.sources }),
  useEntity: (id: number | undefined) => ({ data: id === 9 ? state.source : undefined }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import { EntityMergeDialog } from "@/features/canon/EntityMergeDialog"

const target = {
  id: 5,
  book_id: 1,
  track_id: 3,
  kind: "character",
  name: "Aria",
  aliases: [],
  summary: "",
  description: "",
  state: "",
  attributes: {},
  importance: 4,
  status: "active",
  is_protagonist: false,
  first_chapter_id: null,
  last_chapter_id: null,
  created_at: "",
  updated_at: "",
  relations: [],
  appearances: [],
  recent_changes: [],
  design: null,
} as unknown as EntityDto

const source = {
  id: 9,
  book_id: 1,
  track_id: null,
  kind: "item",
  name: "Bran",
  aliases: [],
  summary: "",
  description: "",
  state: "",
  attributes: {},
  importance: 2,
  status: "active",
  is_protagonist: false,
  first_chapter_id: null,
  last_chapter_id: null,
  created_at: "",
  updated_at: "",
  relations: [],
  appearances: [{ id: 1 }, { id: 2 }],
  recent_changes: [],
  design: null,
}

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

beforeEach(() => {
  mergeMutate.mockReset()
  navigate.mockReset()
  toastSuccess.mockReset()
  // `useEntities` is shared-only, like the list endpoint without a track scope.
  state.shared = [
    { id: 9, track_id: null, kind: "item", name: "Bran" },
    { id: 11, track_id: null, kind: "location", name: "Cave" },
  ]
  state.sources = [
    { id: 5, track_id: 3, kind: "character", name: "Aria" },
    { id: 9, track_id: null, kind: "item", name: "Bran" },
    { id: 11, track_id: null, kind: "location", name: "Cave" },
    { id: 13, track_id: 3, kind: "item", name: "Private relic" },
    { id: 15, track_id: 4, kind: "item", name: "Other track relic" },
  ]
  state.source = source
})

function renderDialog() {
  render(<EntityMergeDialog bookId={1} target={target} open onOpenChange={vi.fn()} />)
}

describe("EntityMergeDialog", () => {
  it("excludes the target entity and other tracks' private entities", () => {
    renderDialog()
    const group = screen.getByRole("radiogroup", { name: "Entity to merge in" })
    const choices = within(group)
      .getAllByRole("radio")
      .map((choice) => choice.textContent)
    expect(choices).toHaveLength(3)
    expect(choices[0]).toContain("Bran")
    expect(choices[1]).toContain("Cave")
    expect(choices[2]).toContain("Private relic")
    expect(within(group).queryByText("Aria")).toBeNull()
    expect(within(group).queryByText("Other track relic")).toBeNull()
  })

  it("offers a private entity as a source for a shared target", () => {
    render(
      <EntityMergeDialog
        bookId={1}
        target={{ ...target, track_id: null } as EntityDto}
        open
        onOpenChange={vi.fn()}
      />,
    )
    const group = screen.getByRole("radiogroup", { name: "Entity to merge in" })
    const names = within(group)
      .getAllByRole("radio")
      .map((choice) => choice.textContent)
    // A shared target can absorb a private entity from any isolated track (spec/06 §9).
    expect(names.some((name) => name?.includes("Private relic"))).toBe(true)
    expect(names.some((name) => name?.includes("Other track relic"))).toBe(true)
  })

  it("names the source from the picker before its detail loads", async () => {
    state.source = undefined
    renderDialog()
    const user = userEvent.setup()
    await user.click(screen.getByRole("radio", { name: /Bran/ }))
    expect(
      screen.getByRole("button", { name: "Merge «Bran» into «Aria»" }),
    ).toBeInTheDocument()
  })

  it("shows the union warning and the shared-into-isolated explanation", async () => {
    renderDialog()
    expect(
      screen.getByText(/A merge cannot be reverted as a whole/),
    ).toBeInTheDocument()
    expect(
      screen.getByText(/A shared entity cannot be merged into a private one/),
    ).toBeInTheDocument()

    const user = userEvent.setup()
    await user.click(screen.getByRole("radio", { name: /Bran/ }))
    expect(
      screen.getByText(
        "A shared entity cannot be merged into an isolated track's private entity. Merge it into a shared entity instead.",
      ),
    ).toBeInTheDocument()
  })

  it("sends {from_id} and moves the route to the survivor", async () => {
    mergeMutate.mockImplementation(
      (_variables: unknown, options: { onSuccess?: (entity: EntityDto) => void }) => {
        options.onSuccess?.(target)
      },
    )
    renderDialog()
    const user = userEvent.setup()
    await user.click(screen.getByRole("radio", { name: /Bran/ }))
    await user.click(screen.getByRole("button", { name: "Merge «Bran» into «Aria»" }))

    expect(mergeMutate.mock.calls[0][0]).toEqual({ id: 5, fromId: 9 })
    expect(navigate).toHaveBeenCalledWith("/books/1/canon/entities/5")
    expect(toastSuccess).toHaveBeenCalledWith("Merged «Bran» into «Aria»")
  })
})
