/**
 * Timeline rows carry a chapter link and entity chips (PROJECT §8.8).
 *
 * The row shows the chapter reference (resolved to its number, D-054) as a link to the chapter
 * page, and the event's entities as chips.
 */
import { render, screen } from "@testing-library/react"
import { beforeAll, describe, expect, it, vi } from "vitest"
import i18n from "i18next"

import enCanon from "@/i18n/locales/en/canon.json"

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
  useTimeline: () => ({
    isPending: false,
    data: [
      {
        id: 12,
        book_id: 1,
        track_id: null,
        position: 1024,
        story_time: "Day 1",
        title: "The gate",
        description: "It opens.",
        chapter_id: 3,
        importance: 3,
        entity_ids: [7],
        created_at: "",
        updated_at: "",
      },
    ],
  }),
  useEntities: () => ({
    data: [{ id: 7, name: "Lin Fan" }],
  }),
  useCreateTimelineEvent: () => ({ mutate: vi.fn() }),
  useDeleteTimelineEvent: () => ({ mutate: vi.fn() }),
  useMoveTimelineEvent: () => ({ mutate: vi.fn() }),
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

import TimelinePage from "@/features/canon/TimelinePage"

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

describe("TimelinePage", () => {
  it("links the chapter and shows the entities as chips", () => {
    render(<TimelinePage />)
    expect(screen.getByRole("link", { name: "Chapter 1" })).toHaveAttribute(
      "href",
      "/books/1/chapters/3",
    )
    expect(screen.getByText("Lin Fan")).toBeInTheDocument()
  })
})
