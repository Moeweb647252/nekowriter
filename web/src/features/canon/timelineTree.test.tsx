/** The timeline as a tree of history and plan (PROJECT §8.8, D-170). */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, describe, expect, it, vi } from "vitest"

import enCanon from "@/i18n/locales/en/canon.json"

const createMutate = vi.hoisted(() => vi.fn())

vi.mock("react-router", async () => {
  const React = await import("react")
  return {
    useParams: () => ({ bookId: "1" }),
    Link: ({ to, children }: { to: string; children: React.ReactNode }) =>
      React.createElement("a", { href: to }, children),
  }
})

vi.mock("@/api/hooks/useCanon", () => ({
  useTimeline: () => ({
    isPending: false,
    data: [
      {
        id: 1, book_id: 1, track_id: null, position: 0, story_time: "",
        title: "", description: "", chapter_id: null, parent_id: null,
        kind: "root", status: "occurred", child_count: 2, importance: 2,
        entity_ids: [], created_at: "", updated_at: "",
      },
      {
        id: 2, book_id: 1, track_id: null, position: 1024, story_time: "",
        title: "Phase", description: "", chapter_id: null, parent_id: null,
        kind: "entry", status: "planned", child_count: 1, importance: 3,
        entity_ids: [], created_at: "", updated_at: "",
      },
      {
        id: 3, book_id: 1, track_id: null, position: 2048, story_time: "",
        title: "Scene", description: "", chapter_id: null, parent_id: 2,
        kind: "entry", status: "occurred", child_count: 0, importance: 2,
        entity_ids: [], created_at: "", updated_at: "",
      },
      {
        id: 4, book_id: 1, track_id: null, position: 3072, story_time: "",
        title: "Dropped", description: "", chapter_id: null, parent_id: null,
        kind: "entry", status: "abandoned", child_count: 0, importance: 2,
        entity_ids: [], created_at: "", updated_at: "",
      },
    ],
  }),
  useEntities: () => ({ data: [] }),
  useCreateTimelineEvent: () => ({ mutate: createMutate }),
  useDeleteTimelineEvent: () => ({ mutate: vi.fn() }),
  useMoveTimelineEvent: () => ({ mutate: vi.fn() }),
  usePatchTimelineEvent: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/api/hooks/tracks", () => ({
  useTracks: () => ({ data: [{ id: 1, kind: "main", title: "" }] }),
}))
vi.mock("@/features/canon/chapterIndex", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/features/canon/chapterIndex")>()
  return { ...actual, useChapterIndex: () => new Map() }
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

import TimelinePage from "@/features/canon/TimelinePage"

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

describe("TimelinePage tree", () => {
  it("renders the tree, planned badge, abandoned strike-through and never the root", () => {
    render(<TimelinePage />)
    expect(screen.queryByText("root")).not.toBeInTheDocument()
    expect(screen.getByText("Phase")).toBeInTheDocument()
    const scene = screen.getByText("Scene").closest("div[style]")!
    expect(scene).toHaveStyle({ marginLeft: "16px" })
    expect(screen.getByText("Planned")).toBeInTheDocument()
    expect(screen.getByText("Dropped").closest("div.line-through")).toBeInTheDocument()
  })

  it("collapses and expands a parent's children", async () => {
    render(<TimelinePage />)
    const user = userEvent.setup()
    expect(screen.getByText("Scene")).toBeInTheDocument()
    await user.click(screen.getAllByRole("button", { name: "Collapse" })[0])
    expect(screen.queryByText("Scene")).not.toBeInTheDocument()
    await user.click(screen.getAllByRole("button", { name: "Expand" })[0])
    expect(screen.getByText("Scene")).toBeInTheDocument()
  })

  it("creates a child from a node with its parent id", async () => {
    render(<TimelinePage />)
    const user = userEvent.setup()
    await user.click(screen.getAllByRole("button", { name: "Add sub-entry" })[0])
    await user.type(screen.getByPlaceholderText("Event title"), "New scene")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(createMutate.mock.calls[0][0]).toMatchObject({
      title: "New scene",
      parent_id: 2,
    })
  })
})
