/**
 * Manually resolving a foreshadowing item stores the author's own words (spec/06 §1.5, D-032).
 *
 * The resolution is written text shown to models by `list_foreshadowing`; the UI must ask for it
 * instead of storing a localized phrase in the canon (the old "Resolved by the author").
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeAll, describe, expect, it, vi } from "vitest"
import i18n from "i18next"

import enCanon from "@/i18n/locales/en/canon.json"

const patchMutate = vi.fn()

vi.mock("react-router", async () => {
  const React = await import("react")
  return {
    useParams: () => ({ bookId: "1" }),
    Link: ({
      to,
      children,
      ...props
    }: {
      to: string
      children: React.ReactNode
    }) => React.createElement("a", { href: to, ...props }, children),
  }
})

vi.mock("@/api/hooks/useCanon", () => ({
  useForeshadowing: () => ({
    isPending: false,
    data: [
      {
        id: 9,
        book_id: 1,
        track_id: null,
        title: "The broken hilt",
        description: "A hilt with no blade.",
        planned_payoff: "The blade returns.",
        payoff_event_id: 12,
        payoff_position: "after",
        payoff_event_title: "The gate",
        payoff_event_status: "planned",
        status: "open",
        planted_chapter_id: 3,
        resolved_chapter_id: null,
        resolution: "",
        importance: 3,
        entity_ids: [],
        created_at: "",
        updated_at: "",
      },
    ],
  }),
  useCreateForeshadowing: () => ({ mutate: vi.fn() }),
  usePatchForeshadowing: () => ({ mutate: patchMutate }),
  useDeleteForeshadowing: () => ({ mutate: vi.fn() }),
}))

vi.mock("@/api/hooks/tracks", () => ({
  useTracks: () => ({ data: [{ id: 1, kind: "main", title: "" }] }),
}))

vi.mock("@/features/canon/chapterIndex", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("@/features/canon/chapterIndex")>()
  return {
    ...actual,
    useChapterIndex: () =>
      new Map([
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
      ]),
  }
})

vi.mock("@/api/hooks/books", () => ({
  useBook: () => ({
    data: { settings: { canon_systems: { goals: false } } },
    isPending: false,
    isError: false,
  }),
}))

vi.mock("@/api/hooks/goals", () => ({
  useGoals: () => ({ data: [], isPending: false, isError: false }),
}))

import ForeshadowingPage from "@/features/canon/ForeshadowingPage"

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

describe("ForeshadowingPage", () => {
  it("asks for the resolution instead of storing UI wording", async () => {
    const user = userEvent.setup()
    render(<ForeshadowingPage />)
    // The planted chapter is a link to the chapter page (PROJECT §8.8).
    expect(screen.getByRole("link", { name: /Planted in Chapter 1/ })).toHaveAttribute(
      "href",
      "/books/1/chapters/3",
    )
    await user.click(screen.getByRole("button", { name: "Resolve" }))
    expect(
      patchMutate,
      "clicking Resolve alone must not store a generated resolution",
    ).not.toHaveBeenCalled()
    const input = screen.getByPlaceholderText("How was it paid off?")
    await user.type(input, "The blade returns.")
    await user.selectOptions(
      screen.getByLabelText("Paid off in chapter"),
      screen.getByRole("option", { name: "Chapter 1" }),
    )
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(patchMutate).toHaveBeenCalledTimes(1)
    const [variables] = patchMutate.mock.calls[0] as [
      { id: number; patch: Record<string, unknown> },
    ]
    expect(variables.id).toBe(9)
    expect(variables.patch).toEqual({
      status: "resolved",
      resolution: "The blade returns.",
      resolved_chapter_id: 3,
    })
  })
})
