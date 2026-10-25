/**
 * The side-story UI (PLAN M10.3, PROJECT §8.4/F9, spec/08 §6):
 * - the New side story dialog derives the spec defaults (isolated; `after_chapter` on the latest
 *   official main chapter, `before_start` when there is none) and sends them;
 * - the track page locks the canon mode once the side story has an official chapter;
 * - the dynamic locale keys (anchor kinds, canon modes, anchor notices) exist in both locales.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeAll, describe, expect, it, vi } from "vitest"
import i18n from "i18next"

import enBook from "@/i18n/locales/en/book.json"
import zhBook from "@/i18n/locales/zh-CN/book.json"
import enCanon from "@/i18n/locales/en/canon.json"
import zhCanon from "@/i18n/locales/zh-CN/canon.json"

const createTrack = vi.fn()
const patchTrack = vi.fn()
const deleteTrack = vi.fn()
const createChapter = vi.fn()
const navigate = vi.fn()

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
