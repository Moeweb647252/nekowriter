/** Inline relation editing (spec/06 §1.2, spec/11 §5.5, D-161). */
import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import enCanon from "@/i18n/locales/en/canon.json"
import type { EntityDto } from "@/api/gen/EntityDto"
import type { RelationDto } from "@/api/gen/RelationDto"

const patchMutate = vi.hoisted(() => vi.fn())

vi.mock("@/api/hooks/useCanon", () => ({
  useEntities: () => ({
    data: [
      { id: 2, name: "Bran" },
      { id: 3, name: "Cave" },
    ],
  }),
  useCreateRelation: () => ({ mutate: vi.fn(), isPending: false }),
  useDeleteRelation: () => ({ mutate: vi.fn(), isPending: false }),
  usePatchRelation: () => ({ mutate: patchMutate, isPending: false }),
  useEntity: () => ({ data: undefined }),
  useEntityHistory: () => ({ data: undefined, isPending: false }),
  usePatchEntity: () => ({ mutate: vi.fn(), isPending: false }),
  useDeleteEntity: () => ({ mutate: vi.fn(), isPending: false }),
  useSetProtagonist: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import { RelationsEditor } from "@/features/canon/EntityDetail"

const relation: RelationDto = {
  id: 9,
  book_id: 1,
  track_id: null,
  source_id: 1,
  target_id: 2,
  kind: "ally",
  inverse_kind: "ally",
  description: "",
  status: "active",
  since_chapter_id: null,
  until_chapter_id: null,
  created_at: "",
  updated_at: "",
}

function entityWith(overrides: Partial<RelationDto> = {}): EntityDto {
  return {
    id: 1,
    book_id: 1,
    track_id: null,
    kind: "character",
    name: "Aria",
    aliases: [],
    summary: "",
    description: "",
    state: "",
    attributes: {},
    importance: 3,
    status: "active",
    is_protagonist: false,
    first_chapter_id: null,
    last_chapter_id: null,
    created_at: "",
    updated_at: "",
    relations: [{ ...relation, ...overrides }],
    appearances: [],
    recent_changes: [],
    design: null,
  } as EntityDto
}

const chapterIndex = new Map([
  [
    4,
    {
      id: 4,
      number: 1,
      title: "One",
      trackId: 1,
      trackKind: "main" as const,
      trackTitle: "",
    },
  ],
])

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

beforeEach(() => {
  patchMutate.mockReset()
})

function renderEditor(entity = entityWith()) {
  render(<RelationsEditor bookId={1} entity={entity} chapterIndex={chapterIndex} />)
}

describe("relation editing", () => {
  it("sends only the changed fields and the note", async () => {
    renderEditor()
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Edit" }))
    await user.type(screen.getByLabelText("Description"), "sworn allies")
    await user.type(screen.getByLabelText("Change note"), "after the duel")
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(patchMutate).toHaveBeenCalledWith(
      { id: 9, patch: { description: "sworn allies", note: "after the duel" } },
      expect.any(Object),
    )
  })

  it("shows the ended label and offers both status labels", async () => {
    renderEditor(entityWith({ status: "ended" }))
    expect(screen.getByText("(Ended)")).toBeInTheDocument()

    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Edit" }))
    const status = screen.getByLabelText("Status")
    expect(within(status).getByRole("option", { name: "Active" })).toBeInTheDocument()
    expect(within(status).getByRole("option", { name: "Ended" })).toBeInTheDocument()
  })

  it("restores the row unchanged on cancel", async () => {
    renderEditor()
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Edit" }))
    await user.clear(screen.getByLabelText("Kind"))
    await user.type(screen.getByLabelText("Kind"), "enemy")
    await user.click(screen.getByRole("button", { name: "Cancel" }))

    expect(patchMutate).not.toHaveBeenCalled()
    expect(screen.getByText("ally")).toBeInTheDocument()
    expect(screen.queryByText("enemy")).toBeNull()
  })
})
