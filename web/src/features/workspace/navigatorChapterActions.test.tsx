/**
 * Chapter actions in the navigator (spec/08 §1.1, spec/11 §5.1, PROJECT §8.4):
 * - the Delete item on a chapter row asks before it destroys anything, and cancelling keeps the
 *   chapter — the endpoint existed but nothing ever called it (the reported bug);
 * - the dialog states the consequences, including the canon warning that applies only to an
 *   official chapter;
 * - the two refusals the server can answer (`chapter.session_exists`, `chapter.is_anchor`) are
 *   shown inside the dialog with the names to act on, not as a bare error toast;
 * - the menu is reachable by keyboard and ignores an IME composition (AGENTS.md §6);
 * - "+ Next chapter" is disabled, with a reason, while the book is not `active` (a chapter created
 *   before the conception is finalized would be written against an empty book).
 */
import { fireEvent, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import { ApiError } from "@/api/client"

const state = vi.hoisted(() => ({
  deleteMutate: vi.fn(),
  createMutate: vi.fn(),
  navigate: vi.fn(),
  /** The main track's chapter rows. */
  chapters: [] as unknown[],
  /** The side track's chapter rows; empty unless a test needs one. */
  sideChapters: [] as unknown[],
}))

vi.mock("react-router", () => ({
  Link: ({ children, to }: { children: React.ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
  NavLink: ({ children, to }: { children: React.ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
  useNavigate: () => state.navigate,
  useParams: () => ({ bookId: "1" }),
}))

vi.mock("@/api/hooks/reviews", () => ({
  useProposals: () => ({ data: [] }),
}))

vi.mock("@/api/hooks/tracks", () => ({
  useTracks: () => ({
    data: [
      { id: 1, kind: "main", title: "", canon_mode: "canon" },
      { id: 5, kind: "side", title: "旧事", canon_mode: "isolated" },
    ],
  }),
  // Only the main track (id 1) has chapters; the side track stays empty so each row appears once.
  useChapters: (trackId: number) => ({ data: trackId === 1 ? state.chapters : state.sideChapters }),
  useCreateChapter: () => ({ mutate: state.createMutate, isPending: false }),
  useReorderChapters: () => ({ mutate: vi.fn() }),
}))

vi.mock("@/api/hooks/chapters", () => ({
  useDeleteChapter: () => ({ mutate: state.deleteMutate, isPending: false }),
}))

vi.mock("@/components/common/ErrorToast", () => ({
  useErrorToast: () => () => {},
  useWarningToasts: () => () => {},
}))

vi.mock("@/features/workspace/NewSideStoryDialog", () => ({
  NewSideStoryDialog: () => null,
}))

import type { BookDto } from "@/api/gen/BookDto"
import type { ChapterListItemDto } from "@/api/gen/ChapterListItemDto"
import { Navigator } from "@/features/workspace/Navigator"

/** A book in `status`. */
function book(status: BookDto["status"]): BookDto {
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
      recent_chapter_count: 2,
      midterm_window: 10,
      auto_maintenance: true,
      canon_systems: { entities: true, timeline: true, memory: true, foreshadowing: true, goals: true },
    },
    model_overrides: {},
    conception: { status: "none", revision: 0, topics: [], characters: [] },
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  } as unknown as BookDto
}

/** A chapter in the main track in `status`. */
function chapter(
  id: number,
  number: number,
  status: ChapterListItemDto["status"],
): ChapterListItemDto {
  return {
    id,
    book_id: 1,
    track_id: 1,
    number,
    title: `Chapter ${number}`,
    status,
    canon_status: status === "official" ? "synced" : "none",
    word_count: 10,
    has_outline: false,
    updated_at: "2026-01-01T00:00:00Z",
  }
}

/** Renders the navigator for one book. */
function renderNavigator(status: BookDto["status"] = "active") {
  return render(<Navigator book={book(status)} />)
}

/** Opens the `…` menu of the chapter numbered `number` and returns its Delete item. */
async function openRowMenu(number: number) {
  const user = userEvent.setup()
  await user.click(
    screen.getByRole("button", { name: `Actions for chapter ${number}` }),
  )
  return { user, item: await screen.findByRole("menuitem", { name: "Delete chapter" }) }
}

/** The options object the mutation was given, so a test can drive its callbacks. */
function mutateOptions(): {
  onSuccess: () => void
  onError: (error: unknown) => void
} {
  const options = state.deleteMutate.mock.calls[0]?.[1] as
    | { onSuccess: () => void; onError: (error: unknown) => void }
    | undefined
  expect(options, "the mutation must be given callbacks").toBeDefined()
  return options as { onSuccess: () => void; onError: (error: unknown) => void }
}

/** One "+ Next chapter" button per track, in track order: the main track's is first. */
function nextChapterButtons(): HTMLElement[] {
  return screen.getAllByRole("button", { name: "Next chapter" })
}

beforeEach(() => {
  state.deleteMutate.mockReset()
  state.createMutate.mockReset()
  state.navigate.mockReset()
  state.chapters = [chapter(11, 1, "drafting"), chapter(12, 2, "drafting")]
})

describe("chapter deletion from the navigator", () => {
  it("does not delete anything before the confirmation is accepted", async () => {
    renderNavigator()
    const { user, item } = await openRowMenu(1)
    await user.click(item)

    expect(
      state.deleteMutate,
      "opening the menu and choosing Delete must not delete",
    ).not.toHaveBeenCalled()
    expect(screen.getByText("Delete Chapter 1?")).toBeInTheDocument()

    await user.click(screen.getByRole("button", { name: "Cancel" }))
    expect(state.deleteMutate, "cancelling must keep the chapter").not.toHaveBeenCalled()
    expect(screen.queryByText("Delete Chapter 1?")).not.toBeInTheDocument()
  })

  it("deletes the chapter the menu belongs to once confirmed", async () => {
    renderNavigator()
    const { user, item } = await openRowMenu(2)
    await user.click(item)
    await user.click(screen.getByRole("button", { name: "Delete chapter" }))

    expect(state.deleteMutate).toHaveBeenCalledTimes(1)
    expect(state.deleteMutate.mock.calls[0][0], "the row's own chapter id").toBe(12)
  })

  it("states the consequences of the delete", async () => {
    renderNavigator()
    const { user, item } = await openRowMenu(1)
    await user.click(item)

    expect(
      screen.getByText(
        "The chapter is deleted for good, with its text, versions and review reports.",
      ),
    ).toBeInTheDocument()
    expect(screen.getByText(/Later chapters of this track are renumbered/)).toBeInTheDocument()
  })

  it("warns that canon keeps its records for an official chapter", async () => {
    state.chapters = [chapter(11, 1, "official"), chapter(12, 2, "drafting")]
    renderNavigator()
    const { user, item } = await openRowMenu(1)
    await user.click(item)

    expect(
      screen.getByText(
        "This chapter is official: canon keeps its records, but the chapter references in them become null.",
      ),
    ).toBeInTheDocument()
  })

  it("does not show the canon warning for a non-official chapter", async () => {
    renderNavigator()
    const { user, item } = await openRowMenu(1)
    await user.click(item)

    expect(
      screen.queryByText(
        "This chapter is official: canon keeps its records, but the chapter references in them become null.",
      ),
    ).not.toBeInTheDocument()
  })
})

describe("refusals the delete dialog explains", () => {
  it("names the open session and where to end it", async () => {
    renderNavigator()
    const { user, item } = await openRowMenu(1)
    await user.click(item)
    await user.click(screen.getByRole("button", { name: "Delete chapter" }))

    mutateOptions().onError(
      new ApiError(409, "chapter.session_exists", "the chapter has an open session", {
        session_id: 77,
      }),
    )

    expect(
      await screen.findByText(
        "Agent session #77 is still open on this chapter. End that session, then delete.",
      ),
    ).toBeInTheDocument()
    expect(
      await screen.findByRole("link", { name: "Open the chapter to end the session" }),
    ).toHaveAttribute("href", "/books/1/chapters/11")
    // The primary button no longer promises a delete: it acknowledges the refusal.
    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument()
    expect(state.deleteMutate, "a refused delete is not retried by itself").toHaveBeenCalledTimes(1)
  })

  it("names the side stories a chapter anchors", async () => {
    renderNavigator()
    const { user, item } = await openRowMenu(1)
    await user.click(item)
    await user.click(screen.getByRole("button", { name: "Delete chapter" }))

    mutateOptions().onError(
      new ApiError(409, "chapter.is_anchor", "the chapter anchors a side track", {
        track_ids: [5],
      }),
    )

    expect(
      await screen.findByText("This chapter anchors a side story, so it cannot be deleted yet:"),
    ).toBeInTheDocument()
    // The track list is already loaded by the navigator, so the dialog names the side story
    // instead of printing its id.
    expect(await screen.findByRole("link", { name: "旧事" })).toHaveAttribute(
      "href",
      "/books/1/tracks/5",
    )
    expect(
      screen.getByText(
        "Point each side story below at another anchor chapter, then delete this one.",
      ),
    ).toBeInTheDocument()
  })

  it("sends any other failure to the localized error toast", async () => {
    renderNavigator()
    const { user, item } = await openRowMenu(1)
    await user.click(item)
    await user.click(screen.getByRole("button", { name: "Delete chapter" }))

    mutateOptions().onError(new ApiError(500, "internal", "boom"))

    // A real failure is not a refusal: the dialog keeps its delete button and says nothing about
    // anchors or sessions.
    expect(screen.queryByText(/anchors a side story/)).not.toBeInTheDocument()
    expect(screen.queryByText(/is still open on this chapter/)).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Delete chapter" })).toBeInTheDocument()
  })
})

describe("chapter row menu keyboard support", () => {
  it("opens with Enter, closes with Escape and ignores an IME composition", async () => {
    const user = userEvent.setup()
    renderNavigator()
    const trigger = screen.getByRole("button", { name: "Actions for chapter 1" })

    // A confirming Enter during a composition belongs to the input method, not the menu
    // (AGENTS.md §6). jsdom honours `isComposing` only when it is defined on the event.
    const composing = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })
    Object.defineProperty(composing, "isComposing", { value: true })
    fireEvent(trigger, composing)
    expect(screen.queryByRole("menuitem", { name: "Delete chapter" })).not.toBeInTheDocument()

    trigger.focus()
    await user.keyboard("{Enter}")
    expect(
      await screen.findByRole("menuitem", { name: "Delete chapter" }),
    ).toBeInTheDocument()

    await user.keyboard("{Escape}")
    expect(screen.queryByRole("menuitem", { name: "Delete chapter" })).not.toBeInTheDocument()
  })
})

describe("creating a chapter needs an active book", () => {
  it("disables + Next chapter and points at the conception studio while conceiving", async () => {
    renderNavigator("conceiving")

    const button = nextChapterButtons()[0]
    expect(button).toBeDisabled()
    // The reason appears once per track, so it is asserted as text rather than as one element.
    expect(
      screen.getAllByText(
        "Finish the conception first: a chapter can only be added to a book that is active.",
      ).length,
    ).toBeGreaterThan(0)
    // ...and it says where to go instead: the conception studio (PROJECT §8.7).
    for (const link of screen.getAllByRole("link", { name: "Open the conception studio" })) {
      expect(link).toHaveAttribute("href", "/books/1/conception")
    }

    await userEvent.setup().click(button)
    expect(state.createMutate, "a disabled button must not create anything").not.toHaveBeenCalled()
  })

  it("disables + Next chapter on an archived book without the conception link", () => {
    renderNavigator("archived")

    expect(nextChapterButtons()[0]).toBeDisabled()
    expect(
      screen.getAllByText(
        "This book is archived, so no new chapter can be added. Unarchive it first.",
      ).length,
    ).toBeGreaterThan(0)
    expect(
      screen.queryByRole("link", { name: "Open the conception studio" }),
    ).not.toBeInTheDocument()
  })

  it("keeps existing chapter rows reachable and deletable on a conceiving book", async () => {
    // The reported bug left a chapter on a book whose conception was never finalized, so the row
    // exists in real data. Only *creating* is gated (spec/08 §1.1): gating the rows too would
    // leave the author unable to open or clean up the chapter the bug created.
    state.chapters = [chapter(11, 1, "planned")]
    renderNavigator("conceiving")

    expect(nextChapterButtons()[0]).toBeDisabled()
    // The row is there, links to the chapter and carries its menu.
    expect(screen.getByRole("link", { name: /Chapter 1/ })).toHaveAttribute(
      "href",
      "/books/1/chapters/11",
    )
    const { user, item } = await openRowMenu(1)
    await user.click(item)
    await user.click(screen.getByRole("button", { name: "Delete chapter" }))
    expect(state.deleteMutate).toHaveBeenCalledTimes(1)
    expect(state.deleteMutate.mock.calls[0][0]).toBe(11)
  })

  it("enables + Next chapter on an active book and explains nothing", async () => {
    renderNavigator("active")

    const button = nextChapterButtons()[0]
    expect(button).toBeEnabled()
    expect(screen.queryByText(/Finish the conception first/)).not.toBeInTheDocument()

    await userEvent.setup().click(button)
    expect(state.createMutate).toHaveBeenCalledTimes(1)
  })
})
