/**
 * Retiring a memory note writes no generated change note (spec/06 §3, D-032).
 *
 * `change_log.note` only ever holds written text (the agent's note, the author's own note, a
 * proposal's rationale, `#B → #A`); a localized "Retired by the author" is UI wording and must
 * not become canon data — model-facing renderings include notes.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeAll, describe, expect, it, vi } from "vitest"
import i18n from "i18next"

import enCanon from "@/i18n/locales/en/canon.json"

const patchMutate = vi.fn()

vi.mock("@/api/hooks/useCanon", () => ({
  useMemoryNotes: () => ({
    isPending: false,
    data: [
      {
        id: 5,
        book_id: 1,
        track_id: null,
        kind: "fact",
        content: "The sword was a gift.",
        chapter_id: null,
        importance: 3,
        status: "active",
        entity_ids: [],
        created_at: "",
        updated_at: "",
      },
    ],
  }),
  useCreateMemoryNote: () => ({ mutate: vi.fn() }),
  usePatchMemoryNote: () => ({ mutate: patchMutate }),
  useDeleteMemoryNote: () => ({ mutate: vi.fn() }),
}))

import { NotesTable } from "@/features/canon/NotesPage"

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

describe("NotesTable", () => {
  it("retires a note without inventing a change note", async () => {
    const user = userEvent.setup()
    render(<NotesTable bookId={1} />)
    await user.click(screen.getByRole("button", { name: "Retire" }))
    expect(patchMutate).toHaveBeenCalledTimes(1)
    const [variables] = patchMutate.mock.calls[0] as [
      { id: number; patch: Record<string, unknown> },
    ]
    expect(variables.id).toBe(5)
    expect(variables.patch).toEqual({ status: "retired" })
    expect(variables.patch).not.toHaveProperty("note")
  })
})
