/** The timeline as a tree of history and plan (PROJECT §8.8, D-170). */
import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import enCanon from "@/i18n/locales/en/canon.json"
import { ApiError } from "@/api/client"
import type { TimelineEventDto } from "@/api/gen/TimelineEventDto"
import { siblingMove } from "@/features/canon/timelineTree"

const createMutate = vi.hoisted(() => vi.fn())
const deleteMutate = vi.hoisted(() => vi.fn())
const patchMutate = vi.hoisted(() => vi.fn())
const toastError = vi.hoisted(() => vi.fn())

vi.mock("react-router", async () => {
  const React = await import("react")
  return {
    useParams: () => ({ bookId: "1" }),
    Link: ({ to, children }: { to: string; children: React.ReactNode }) =>
      React.createElement("a", { href: to }, children),
  }
})

vi.mock("sonner", () => ({
  toast: { error: toastError, warning: vi.fn(), success: vi.fn() },
}))

vi.mock("@/api/hooks/useCanon", () => ({
  useTimeline: () => ({
    isPending: false,
    data: [
      {
        id: 1, book_id: 1, track_id: null, position: 0, story_time: "",
        title: "ROOT SHOULD NOT RENDER", description: "", chapter_id: null, parent_id: null,
        kind: "root", status: "occurred", child_count: 3, importance: 2,
        entity_ids: [], created_at: "", updated_at: "",
      },
      {
        id: 2, book_id: 1, track_id: null, position: 1024, story_time: "",
        title: "Phase", description: "", chapter_id: null, parent_id: null,
        kind: "entry", status: "planned", child_count: 2, importance: 3,
        entity_ids: [], created_at: "", updated_at: "",
      },
      {
        id: 3, book_id: 1, track_id: null, position: 2048, story_time: "",
        title: "Scene", description: "", chapter_id: null, parent_id: 2,
        kind: "entry", status: "occurred", child_count: 0, importance: 2,
        entity_ids: [], created_at: "", updated_at: "",
      },
      {
        id: 5, book_id: 1, track_id: null, position: 2560, story_time: "",
        title: "Planned child", description: "", chapter_id: null, parent_id: 2,
        kind: "entry", status: "planned", child_count: 0, importance: 2,
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
  useDeleteTimelineEvent: () => ({ mutate: deleteMutate }),
  useMoveTimelineEvent: () => ({ mutate: vi.fn() }),
  usePatchTimelineEvent: () => ({ mutate: patchMutate, isPending: false }),
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

beforeEach(() => {
  createMutate.mockReset()
  deleteMutate.mockReset()
  patchMutate.mockReset()
  toastError.mockReset()
})

function event(overrides: Partial<TimelineEventDto>): TimelineEventDto {
  return {
    id: 1, book_id: 1, track_id: null, position: 0, story_time: "",
    title: "", description: "", chapter_id: null, parent_id: null,
    kind: "entry", status: "occurred", child_count: 0, importance: 2,
    entity_ids: [], created_at: "", updated_at: "",
    ...overrides,
  }
}

describe("TimelinePage tree", () => {
  it("renders the tree, planned badge, abandoned strike-through and never the root", () => {
    render(<TimelinePage />)
    expect(screen.queryByText("ROOT SHOULD NOT RENDER")).not.toBeInTheDocument()
    expect(screen.getByText("Phase")).toBeInTheDocument()
    const scene = screen.getByText("Scene").closest("div[style]")!
    expect(scene).toHaveStyle({ marginLeft: "16px" })
    expect(screen.getAllByText("Planned").length).toBeGreaterThan(0)
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

  it("creates a child from the row menu with its parent id", async () => {
    render(<TimelinePage />)
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Timeline actions for Phase" }))
    await user.click(screen.getByRole("menuitem", { name: "Add sub-entry" }))
    expect(
      screen.getByRole("heading", { name: "New timeline event" }),
    ).toBeInTheDocument()
    expect(screen.getByLabelText("Parent")).toHaveValue("2")
    await user.type(screen.getByLabelText("Title"), "New scene")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(createMutate.mock.calls[0][0]).toMatchObject({
      title: "New scene",
      parent_id: 2,
    })
  })

  it("offers Mark as happened and Abandon only on planned rows", async () => {
    render(<TimelinePage />)
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Timeline actions for Phase" }))
    expect(screen.getByRole("menuitem", { name: "Mark as happened" })).toBeInTheDocument()
    expect(screen.getByRole("menuitem", { name: "Abandon" })).toBeInTheDocument()
    await user.keyboard("{Escape}")
    await user.click(screen.getByRole("button", { name: "Timeline actions for Scene" }))
    expect(screen.queryByRole("menuitem", { name: "Mark as happened" })).toBeNull()
    expect(screen.queryByRole("menuitem", { name: "Abandon" })).toBeNull()
  })

  it("disables Delete while an entry has sub-entries and names the reason", async () => {
    render(<TimelinePage />)
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Timeline actions for Phase" }))
    expect(screen.getByRole("menuitem", { name: "Delete" })).toHaveAttribute(
      "aria-disabled",
      "true",
    )
    expect(screen.getByRole("menuitem", { name: "Delete" })).toHaveAttribute(
      "title",
      "Delete is unavailable while this entry has 2 sub-entries.",
    )
    await user.keyboard("{Escape}")
    await user.click(screen.getByRole("button", { name: "Timeline actions for Scene" }))
    expect(screen.getByRole("menuitem", { name: "Delete" })).not.toHaveAttribute(
      "aria-disabled",
      "true",
    )
  })

  it("names the planned sub-entries in the Abandon confirmation", async () => {
    render(<TimelinePage />)
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Timeline actions for Phase" }))
    await user.click(screen.getByRole("menuitem", { name: "Abandon" }))
    const dialog = screen.getByRole("alertdialog")
    expect(within(dialog).getByText("Abandon this entry?")).toBeInTheDocument()
    expect(within(dialog).getByText("Planned child")).toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "Abandon" }))
    expect(patchMutate).toHaveBeenCalledWith(
      {
        id: 2,
        patch: {
          status: "abandoned",
          note: "Abandoned from the timeline",
        },
      },
      expect.anything(),
    )
  })

  it("surfaces a timeline.has_children refusal on delete", async () => {
    deleteMutate.mockImplementationOnce(
      (_id: number, options: { onError?: (error: unknown) => void }) => {
        options.onError?.(
          new ApiError(409, "timeline.has_children", "children", {
            children_count: 2,
          }),
        )
      },
    )
    render(<TimelinePage />)
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Timeline actions for Scene" }))
    await user.click(screen.getByRole("menuitem", { name: "Delete" }))
    expect(toastError).toHaveBeenCalledWith(
      "This entry still has 2 sub-entries. Move or delete them first.",
    )
  })

  it("moves only among siblings and never changes the parent", () => {
    const parent = event({ id: 2, title: "Phase", position: 1024 })
    const first = event({ id: 3, title: "Scene", parent_id: 2, position: 2048 })
    const second = event({ id: 5, title: "Planned child", parent_id: 2, position: 2560 })
    const outside = event({ id: 6, title: "Outside", parent_id: null, position: 3000 })
    expect(siblingMove(first, second, [first, second])).toEqual({ afterId: 5 })
    expect(siblingMove(second, first, [first, second])).toEqual({ beforeId: 3 })
    expect(siblingMove(first, outside, [first, outside])).toBeNull()
    expect(parent.id).toBe(2)
  })
  it("hides abandoned rows when Show abandoned is off", async () => {
    render(<TimelinePage />)
    const user = userEvent.setup()
    expect(screen.getByText("Dropped")).toBeInTheDocument()
    await user.click(screen.getByRole("checkbox", { name: "Show abandoned" }))
    expect(screen.queryByText("Dropped")).toBeNull()
  })

  it("inserts at the top edge before the first row", async () => {
    render(<TimelinePage />)
    const user = userEvent.setup()
    await user.click(screen.getAllByRole("button", { name: "Insert here" })[0])
    await user.type(screen.getByPlaceholderText("Event title"), "Prologue")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(createMutate.mock.calls[0][0]).toMatchObject({
      title: "Prologue",
      before_id: 2,
    })
  })
})
