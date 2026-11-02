/**
 * The entity table's "last seen" cell (PROJECT §8.8, D-054).
 *
 * A chapter reference is its number, not the chapter row's id: the page must resolve the id
 * through the book's chapter lists. Chapter id 3 is chapter number 1 here (a second book), so the
 * old rendering "Chapter 3" is exactly the visible symptom.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeAll, describe, expect, it, vi } from "vitest"
import i18n from "i18next"

import enCanon from "@/i18n/locales/en/canon.json"

const chapterIndex = new Map([
  [
    3,
    {
      id: 3,
      number: 1,
      title: "One",
      trackId: 1,
      trackKind: "main" as const,
      trackTitle: "",
    },
  ],
])

vi.mock("react-router", () => ({
  useParams: () => ({ bookId: "1" }),
  useNavigate: () => vi.fn(),
}))

vi.mock("@/api/hooks/useCanon", () => ({
  useEntities: () => ({
    isPending: false,
    data: [
      {
        id: 7,
        book_id: 1,
        track_id: null,
        kind: "character",
        name: "Lin Fan",
        aliases: [],
        summary: "",
        importance: 3,
        status: "active",
        is_protagonist: false,
        first_chapter_id: 3,
        last_chapter_id: 3,
        created_at: "",
        updated_at: "",
      },
    ],
  }),
  useSetProtagonist: () => ({ mutate: vi.fn() }),
  useCreateEntity: () => ({ mutate: vi.fn(), isPending: false }),
}))

vi.mock("@/api/hooks/tracks", () => ({
  useTracks: () => ({
    data: [
      { id: 1, kind: "main", title: "", canon_mode: "canon" },
      { id: 3, kind: "side", title: "Side", canon_mode: "isolated" },
    ],
  }),
}))

vi.mock("@/features/canon/chapterIndex", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("@/features/canon/chapterIndex")>()
  return { ...actual, useChapterIndex: () => chapterIndex }
})

import EntitiesPage from "@/features/canon/EntitiesPage"

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

describe("EntitiesPage", () => {
  it("shows the chapter number of the last appearance", () => {
    render(<EntitiesPage />)
    expect(screen.getByText("Chapter 1")).toBeInTheDocument()
    expect(screen.queryByText("Chapter 3")).not.toBeInTheDocument()
  })

  it("opens the create dialog from the filter bar", async () => {
    const user = userEvent.setup()
    render(<EntitiesPage />)
    await user.click(screen.getByRole("button", { name: "New entity" }))
    expect(screen.getByLabelText("Kind")).toBeInTheDocument()
  })
})
