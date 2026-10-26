/**
 * The side-story UI (PLAN M10.3, PROJECT §8.4/F9, spec/08 §6):
 * - the New side story dialog derives the spec defaults (isolated; `after_chapter` on the latest
 *   official main chapter, `before_start` when there is none) and sends them;
 * - the track page locks the canon mode once the side story has an official chapter;
 * - "Next chapter" is disabled, with a reason, while the book is not `active` (spec/08 §1.1,
 *   D-130) — the same rule the navigator states — while the chapters that already exist stay
 *   listed and openable;
 * - the dynamic locale keys (anchor kinds, canon modes, anchor notices) exist in both locales.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest"
import i18n from "i18next"

import type { BookDto } from "@/api/gen/BookDto"
import type { BookStatus } from "@/api/gen/BookStatus"
import enBook from "@/i18n/locales/en/book.json"
import zhBook from "@/i18n/locales/zh-CN/book.json"
import enCanon from "@/i18n/locales/en/canon.json"
import zhCanon from "@/i18n/locales/zh-CN/canon.json"

const createTrack = vi.fn()
const patchTrack = vi.fn()
const deleteTrack = vi.fn()
const createChapter = vi.fn()
const navigate = vi.fn()

/** The book `BookLayout` provides through the outlet context, in the status under test. */
let outletBook: BookDto

/** A book in `status` (only the fields the page reads are meaningful). */
function book(status: BookStatus): BookDto {
  return {
    id: 1,
    title: "青云劫",
    idea: "an idea",
    genre: "xianxia",
    writing_language: "zh-CN",
    status,
    settings: {
      granularity: "paragraph",
      auto_pause: false,
      pause_after_outline: true,
      target_words_min: 3000,
      target_words_max: 4000,
      recent_summary_count: 5,
      midterm_window: 10,
      auto_maintenance: true,
    },
    model_overrides: {},
    conception: {
      session_id: null,
      characters_revision: 0,
      characters_approved_revision: null,
      topics_revision: 0,
      topics_approved_revision: null,
      finalized_at: null,
    },
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  }
}

const mainChapters = [
  {
    id: 11,
    book_id: 1,
    track_id: 1,
    number: 1,
    title: "One",
    status: "official",
    canon_status: "synced",
    word_count: 10,
    updated_at: "2026-01-01T00:00:00Z",
  },
  {
    id: 12,
    book_id: 1,
    track_id: 1,
    number: 2,
    title: "Two",
    status: "official",
    canon_status: "synced",
    word_count: 10,
    updated_at: "2026-01-01T00:00:00Z",
  },
  {
    id: 13,
    book_id: 1,
    track_id: 1,
    number: 3,
    title: "Three",
    status: "drafting",
    canon_status: "none",
    word_count: 10,
    updated_at: "2026-01-01T00:00:00Z",
  },
]

let trackChapters: typeof mainChapters = []

vi.mock("react-router", () => ({
  useParams: () => ({ bookId: "1", trackId: "2" }),
  useNavigate: () => navigate,
  // The page reads the book from `BookLayout`'s outlet context instead of loading it again (its
  // tests do not mount the layout, so the context is stubbed here).
  useOutletContext: () => ({ book: outletBook }),
  Link: ({ children, to }: { children: React.ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}))

vi.mock("@/api/hooks/tracks", () => ({
  useTracks: () => ({
    data: [
      { id: 1, kind: "main", title: "", canon_mode: "canon" },
      {
        id: 2,
        kind: "side",
        title: "旧事",
        canon_mode: "isolated",
        anchor_kind: "after_chapter",
        anchor_chapter_id: 12,
        focus_entity_id: null,
        synopsis: "",
        updated_at: "2026-01-01T00:00:00Z",
      },
    ],
  }),
  useTrack: () => ({
    isPending: false,
    data: {
      id: 2,
      book_id: 1,
      kind: "side",
      title: "旧事",
      synopsis: "以前的事",
      canon_mode: "isolated",
      anchor_kind: "after_chapter",
      anchor_chapter_id: 12,
      focus_entity_id: null,
      ord: 1,
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
    },
  }),
  useChapters: (trackId: number) => ({
    data: trackId === 1 ? mainChapters : trackChapters,
  }),
  useCreateTrack: () => ({ mutate: createTrack, isPending: false }),
  usePatchTrack: () => ({ mutate: patchTrack, isPending: false }),
  useDeleteTrack: () => ({ mutate: deleteTrack, isPending: false }),
  useCreateChapter: () => ({ mutate: createChapter, isPending: false }),
}))

vi.mock("@/features/canon/chapterIndex", () => ({
  useChapterIndex: () => new Map(),
  chapterRefLabel: (_t: unknown, _index: unknown, id: number) => `Chapter ${id}`,
}))

vi.mock("@/api/hooks/useCanon", () => ({
  useEntities: () => ({
    data: [
      { id: 7, name: "林凡", kind: "character" },
      { id: 8, name: "张三", kind: "character" },
    ],
  }),
}))

import { NewSideStoryDialog } from "@/features/workspace/NewSideStoryDialog"
import SideStoryPage from "@/features/workspace/SideStoryPage"

beforeAll(() => {
  i18n.addResourceBundle("en", "book", enBook, true, true)
  i18n.addResourceBundle("zh-CN", "book", zhBook, true, true)
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
  i18n.addResourceBundle("zh-CN", "canon", zhCanon, true, true)
  i18n.changeLanguage("en")
})

beforeEach(() => {
  createChapter.mockReset()
  navigate.mockReset()
  outletBook = book("active")
  trackChapters = []
})

describe("side stories", () => {
  it("derives the latest official main chapter as the default anchor", async () => {
    render(<NewSideStoryDialog bookId={1} open onOpenChange={() => {}} />)
    const user = userEvent.setup()
    // The anchor kind defaults to after_chapter; chapter 2 is the latest official chapter, and
    // chapter 3 (drafting) is not eligible as a default.
    expect(screen.getByText("After a main chapter")).toBeInTheDocument()
    await user.type(screen.getByLabelText("Title"), "旧事")
    await user.type(screen.getByLabelText("Premise"), "以前的事")
    await user.click(screen.getByRole("button", { name: "Create side story" }))
    expect(createTrack).toHaveBeenCalledTimes(1)
    expect(createTrack.mock.calls[0][0]).toMatchObject({
      title: "旧事",
      synopsis: "以前的事",
      canon_mode: "isolated",
      anchor_kind: "after_chapter",
      anchor_chapter_id: 12,
      focus_entity_id: undefined,
    })
  })

  it("locks the canon mode once the side story has an official chapter", () => {
    trackChapters = [
      {
        id: 21,
        book_id: 1,
        track_id: 2,
        number: 1,
        title: "一",
        status: "official",
        canon_status: "synced",
        word_count: 10,
        updated_at: "2026-01-01T00:00:00Z",
      },
    ]
    render(<SideStoryPage />)
    expect(
      screen.getByText(
        "The canon mode is locked: this side story already has an official chapter.",
      ),
    ).toBeInTheDocument()
    trackChapters = []
  })

  it("has the dynamic anchor and canon-mode keys in both locales", () => {
    for (const table of [enBook, zhBook]) {
      for (const kind of ["after_chapter", "before_start", "after_end", "none"]) {
        expect(table[`anchor.${kind}` as keyof typeof table]).toBeTypeOf("string")
      }
      for (const mode of ["isolated", "canon"]) {
        expect(table[`canon_mode.${mode}` as keyof typeof table]).toBeTypeOf("string")
      }
    }
    for (const table of [enCanon, zhCanon]) {
      for (const kind of ["after_chapter", "before_start", "after_end", "none"]) {
        expect(
          table[`anchor_notice.${kind}` as keyof typeof table],
        ).toBeTypeOf("string")
      }
    }
  })
})

describe("creating a chapter needs an active book (spec/08 §1.1, D-130)", () => {
  /** A side-story chapter row: the real-data case the gate must not hide. */
  const existingSideChapter = {
    id: 21,
    book_id: 1,
    track_id: 2,
    number: 1,
    title: "A side chapter",
    status: "planned",
    canon_status: "none",
    word_count: 0,
    updated_at: "2026-01-01T00:00:00Z",
  }

  it("disables Next chapter and points at the conception studio while conceiving", async () => {
    outletBook = book("conceiving")
    render(<SideStoryPage />)

    const button = screen.getByRole("button", { name: "Next chapter" })
    expect(button).toBeDisabled()
    // The reason is the navigator's, word for word.
    expect(
      screen.getByText(
        "Finish the conception first: a chapter can only be added to a book that is active.",
      ),
    ).toBeInTheDocument()
    // ...and it says where to go instead (PROJECT §8.7).
    expect(
      screen.getByRole("link", { name: "Open the conception studio" }),
    ).toHaveAttribute("href", "/books/1/conception")

    await userEvent.setup().click(button)
    expect(createChapter, "a disabled button must not create anything").not.toHaveBeenCalled()
  })

  it("disables Next chapter on an archived book without the conception link", () => {
    outletBook = book("archived")
    render(<SideStoryPage />)

    expect(screen.getByRole("button", { name: "Next chapter" })).toBeDisabled()
    expect(
      screen.getByText("This book is archived, so no new chapter can be added. Unarchive it first."),
    ).toBeInTheDocument()
    expect(
      screen.queryByRole("link", { name: "Open the conception studio" }),
    ).not.toBeInTheDocument()
  })

  it("points assistive technology at the reason it disabled the button for", () => {
    outletBook = book("conceiving")
    render(<SideStoryPage />)

    const button = screen.getByRole("button", { name: "Next chapter" })
    const describedBy = button.getAttribute("aria-describedby")
    expect(describedBy, "a disabled button must be described by its reason").toBeTruthy()
    const reason = document.getElementById(describedBy ?? "")
    expect(reason).not.toBeNull()
    expect(reason).toHaveTextContent(
      "Finish the conception first: a chapter can only be added to a book that is active.",
    )
  })

  it("keeps the chapters that already exist reachable on a non-active book", () => {
    // The author's own book 3 is `conceiving` and already has a chapter, created before the rule
    // existed. Only *creating* is gated: hiding or disabling the row would leave that chapter
    // unreachable, and the track deletable only by guessing its URL.
    trackChapters = [existingSideChapter]
    outletBook = book("conceiving")
    render(<SideStoryPage />)

    expect(screen.getByRole("button", { name: "Next chapter" })).toBeDisabled()
    expect(
      screen.getByRole("link", { name: /A side chapter/ }),
    ).toHaveAttribute("href", "/books/1/chapters/21")
    // The row is not the empty state, and it is not visually marked as unavailable.
    expect(screen.queryByText("No chapters yet")).not.toBeInTheDocument()
    // The rest of the page still works for a non-active book: the side story can be deleted.
    expect(screen.getByRole("button", { name: "Delete side story" })).toBeEnabled()
  })

  it("enables Next chapter on an active book and creates one", async () => {
    outletBook = book("active")
    render(<SideStoryPage />)

    const button = screen.getByRole("button", { name: "Next chapter" })
    expect(button).toBeEnabled()
    expect(screen.queryByText(/Finish the conception first/)).not.toBeInTheDocument()
    expect(screen.queryByRole("link", { name: "Open the conception studio" })).not.toBeInTheDocument()

    await userEvent.setup().click(button)
    expect(createChapter).toHaveBeenCalledTimes(1)
    expect(createChapter.mock.calls[0][0]).toEqual({ title: "", guidance: "" })
  })
})
