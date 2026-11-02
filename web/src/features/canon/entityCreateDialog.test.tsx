/** The hand-create entity dialog (PROJECT §8.8, spec/11 §5.5, D-161). */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { ApiError } from "@/api/client"
import enCanon from "@/i18n/locales/en/canon.json"
import type { EntityDto } from "@/api/gen/EntityDto"
import type { TrackDto } from "@/api/gen/TrackDto"

const createMutate = vi.hoisted(() => vi.fn())
const navigate = vi.hoisted(() => vi.fn())
const showError = vi.hoisted(() => vi.fn())

vi.mock("react-router", () => ({ useNavigate: () => navigate }))
vi.mock("@/api/hooks/useCanon", () => ({
  useCreateEntity: () => ({ mutate: createMutate, isPending: false }),
}))
vi.mock("@/api/hooks/tracks", () => ({
  useTracks: () => ({
    data: [
      { id: 1, kind: "main", title: "", canon_mode: "canon" },
      { id: 2, kind: "side", title: "Canon side", canon_mode: "canon" },
      { id: 3, kind: "side", title: "Isolated side", canon_mode: "isolated" },
    ] as unknown as TrackDto[],
  }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => showError }))

import { EntityCreateDialog } from "@/features/canon/EntityCreateDialog"

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

beforeEach(() => {
  createMutate.mockReset()
  navigate.mockReset()
  showError.mockReset()
})

function renderDialog() {
  const onOpenChange = vi.fn()
  render(<EntityCreateDialog bookId={1} open onOpenChange={onOpenChange} />)
  return { onOpenChange }
}

async function fillRequired() {
  const user = userEvent.setup()
  await user.selectOptions(screen.getByLabelText("Kind"), "item")
  await user.type(screen.getByLabelText("Name"), "Test sword")
  return user
}

describe("EntityCreateDialog", () => {
  it("requires a kind and a name", async () => {
    renderDialog()
    const user = userEvent.setup()
    const save = screen.getByRole("button", { name: "Save" })

    await user.click(save)
    expect(createMutate).not.toHaveBeenCalled()

    await user.type(screen.getByLabelText("Name"), "Test sword")
    await user.click(save)
    expect(createMutate).not.toHaveBeenCalled()

    await user.selectOptions(screen.getByLabelText("Kind"), "item")
    await user.click(save)
    expect(createMutate).toHaveBeenCalledTimes(1)
  })

  it("requires a name once a kind is chosen", async () => {
    renderDialog()
    const user = userEvent.setup()
    await user.selectOptions(screen.getByLabelText("Kind"), "item")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(createMutate).not.toHaveBeenCalled()
  })

  it("renders a server field error inline on the named field", async () => {
    createMutate.mockImplementation(
      (_input: unknown, options: { onError?: (error: unknown) => void }) => {
        options.onError?.(
          new ApiError(400, "validation.invalid_fields", "invalid input", {
            fields: { track_id: "not this track" },
          }),
        )
      },
    )
    renderDialog()
    const user = await fillRequired()
    await user.selectOptions(screen.getByLabelText("Storyline"), "3")
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(screen.getByText("Some fields are not valid.")).toBeInTheDocument()
    expect(showError).not.toHaveBeenCalled()
  })

  it("offers only isolated side tracks as a storyline", () => {
    renderDialog()
    const storyline = screen.getByLabelText("Storyline")
    const labels = Array.from(storyline.querySelectorAll("option")).map(
      (option) => option.textContent,
    )
    expect(labels).toEqual(["Shared canon", "Isolated side"])
    expect(screen.queryByRole("option", { name: "Canon side" })).toBeNull()
  })

  it("sends track_id only when one isolated track is chosen", async () => {
    const first = render(<EntityCreateDialog bookId={1} open onOpenChange={vi.fn()} />)
    const user = await fillRequired()
    await user.selectOptions(screen.getByLabelText("Storyline"), "3")
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(createMutate.mock.calls[0][0]).toEqual(
      expect.objectContaining({ kind: "item", name: "Test sword", track_id: 3 }),
    )

    first.unmount()
    createMutate.mockClear()
    render(<EntityCreateDialog bookId={1} open onOpenChange={vi.fn()} />)
    const user2 = await fillRequired()
    await user2.click(screen.getByRole("button", { name: "Save" }))
    const input = createMutate.mock.calls[0][0] as Record<string, unknown>
    expect(input).not.toHaveProperty("track_id")
  })

  it("sends the change note and opens the new entity", async () => {
    createMutate.mockImplementation(
      (_input: unknown, options: { onSuccess?: (entity: EntityDto) => void }) => {
        options.onSuccess?.({ id: 42 } as EntityDto)
      },
    )
    renderDialog()
    const user = await fillRequired()
    await user.type(screen.getByLabelText("Change note"), "First mention")
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(createMutate.mock.calls[0][0]).toEqual(
      expect.objectContaining({ note: "First mention" }),
    )
    expect(navigate).toHaveBeenCalledWith("/books/1/canon/entities/42")
  })
})
