/** The timeline event edit dialog (PROJECT §8.8, spec/11 §5.5, D-161). */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import enCanon from "@/i18n/locales/en/canon.json"
import { ApiError } from "@/api/client"
import type { TimelineEventDto } from "@/api/gen/TimelineEventDto"

const patchMutate = vi.hoisted(() => vi.fn())
const createMutate = vi.hoisted(() => vi.fn())
const state = vi.hoisted(() => ({ goals: true }))

vi.mock("react-router", () => ({
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => (
    <a href={to}>{children}</a>
  ),
}))
vi.mock("@/api/hooks/useCanon", () => ({
  useCreateTimelineEvent: () => ({ mutate: createMutate, isPending: false }),
  usePatchTimelineEvent: () => ({ mutate: patchMutate, isPending: false }),
  useEntities: () => ({
    data: [
      { id: 2, name: "Bran" },
      { id: 3, name: "Cave" },
    ],
  }),
}))
vi.mock("@/api/hooks/books", () => ({
  useBook: () => ({
    data: { settings: { canon_systems: { goals: state.goals } } },
    isPending: false,
    isError: false,
  }),
}))
vi.mock("@/api/hooks/goals", () => ({
  useGoals: () => ({
    data: state.goals ? [{ id: 4, title: "Find the sword" }] : [],
    isPending: false,
    isError: false,
  }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import { TimelineEventDialog } from "@/features/canon/TimelineEventDialog"

const event: TimelineEventDto = {
  id: 9,
  book_id: 1,
  track_id: null,
  position: 3,
  story_time: "Year 1",
  title: "Battle",
  description: "Old description",
  chapter_id: 4,
  parent_id: null,
  kind: "entry",
  status: "occurred",
  child_count: 0,
  importance: 3,
  entity_ids: [2],
  created_at: "",
  updated_at: "",
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
  [
    5,
    {
      id: 5,
      number: 2,
      title: "Two",
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
  createMutate.mockReset()
  state.goals = true
})

function eventWith(overrides: Partial<TimelineEventDto>): TimelineEventDto {
  return { ...event, ...overrides }
}

function renderDialog(overrides: Partial<TimelineEventDto> = {}) {
  render(
    <TimelineEventDialog
      bookId={1}
      event={eventWith(overrides)}
      events={[
        eventWith({ id: 10, title: "Parent candidate", parent_id: null, position: 10 }),
        eventWith({ id: 11, title: "Child candidate", parent_id: 10, position: 11 }),
      ]}
      chapterIndex={chapterIndex}
      open
      onOpenChange={vi.fn()}
    />,
  )
}

describe("TimelineEventDialog", () => {
  it("pre-fills every edited field", () => {
    renderDialog()
    expect(screen.getByLabelText("Title")).toHaveValue("Battle")
    expect(screen.getByLabelText("Description")).toHaveValue("Old description")
    expect(screen.getByLabelText("Story time")).toHaveValue("Year 1")
    expect(screen.getByLabelText("Chapter")).toHaveValue("4")
    expect(
      screen.getByRole("button", { name: "Importance 3 of 5" }),
    ).toHaveAttribute("aria-pressed", "true")
    expect(screen.getByText("Bran", { selector: "span" })).toBeInTheDocument()
  })

  it("does not request a patch when nothing changed", async () => {
    renderDialog()
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(patchMutate).not.toHaveBeenCalled()
  })

  it("saves a changed title only", async () => {
    renderDialog()
    const user = userEvent.setup()
    await user.clear(screen.getByLabelText("Title"))
    await user.type(screen.getByLabelText("Title"), "Skirmish")
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(patchMutate.mock.calls[0][0]).toEqual({
      id: 9,
      patch: { title: "Skirmish" },
    })
  })

  it("saves a changed story time only", async () => {
    renderDialog()
    const user = userEvent.setup()
    await user.clear(screen.getByLabelText("Story time"))
    await user.type(screen.getByLabelText("Story time"), "Year 2")
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(patchMutate.mock.calls[0][0]).toEqual({
      id: 9,
      patch: { story_time: "Year 2" },
    })
  })

  it("saves a changed chapter only", async () => {
    renderDialog()
    const user = userEvent.setup()
    await user.selectOptions(screen.getByLabelText("Chapter"), "5")
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(patchMutate.mock.calls[0][0]).toEqual({
      id: 9,
      patch: { chapter_id: 5 },
    })
  })

  it("saves a changed parent and status", async () => {
    renderDialog()
    const user = userEvent.setup()
    await user.selectOptions(screen.getByLabelText("Parent"), "10")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(patchMutate.mock.calls[0][0]).toEqual({
      id: 9,
      patch: { parent_id: 10 },
    })
  })

  it("limits a planned entry's status to the allowed moves and needs a note", async () => {
    renderDialog({ status: "planned" })
    const user = userEvent.setup()
    const select = screen.getByLabelText("Status")
    expect(
      Array.from(select.querySelectorAll("option")).map((option) => option.textContent),
    ).toEqual(["Planned", "Occurred", "Abandoned"])
    await user.selectOptions(select, "abandoned")
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled()
    await user.type(screen.getByLabelText("Change note"), "the plan was dropped")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(patchMutate.mock.calls[0][0]).toEqual({
      id: 9,
      patch: { status: "abandoned", note: "the plan was dropped" },
    })
  })

  it("saves changed linked entities only", async () => {
    renderDialog()
    const user = userEvent.setup()
    await user.selectOptions(screen.getByLabelText("Linked entities"), "3")
    await user.click(screen.getByRole("button", { name: "Add" }))
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(patchMutate.mock.calls[0][0]).toEqual({
      id: 9,
      patch: { entity_ids: [2, 3] },
    })
  })

  it("saves a changed importance only", async () => {
    renderDialog()
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Importance 4 of 5" }))
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(patchMutate.mock.calls[0][0]).toEqual({
      id: 9,
      patch: { importance: 4 },
    })
  })

  it("sends only the changed fields, the note, and never position", async () => {
    renderDialog()
    const user = userEvent.setup()
    await user.clear(screen.getByLabelText("Description"))
    await user.type(screen.getByLabelText("Description"), "New description")
    await user.type(screen.getByLabelText("Change note"), "after the battle")
    await user.click(screen.getByRole("button", { name: "Save" }))

    const call = patchMutate.mock.calls[0][0] as {
      id: number
      patch: Record<string, unknown>
    }
    expect(call).toEqual({
      id: 9,
      patch: { description: "New description", note: "after the battle" },
    })
    expect(call.patch).not.toHaveProperty("after_id")
    expect(call.patch).not.toHaveProperty("before_id")
  })

  it("clears the chapter to null", async () => {
    renderDialog()
    const user = userEvent.setup()
    await user.selectOptions(screen.getByLabelText("Chapter"), "")
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(patchMutate.mock.calls[0][0]).toEqual({
      id: 9,
      patch: { chapter_id: null },
    })
  })

  it("omits the chapter while it stays null", async () => {
    renderDialog({ chapter_id: null })
    const user = userEvent.setup()
    await user.clear(screen.getByLabelText("Description"))
    await user.type(screen.getByLabelText("Description"), "Still null")
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(patchMutate.mock.calls[0][0]).toEqual({
      id: 9,
      patch: { description: "Still null" },
    })
  })

  it("does not send entities when only their selection order changed", async () => {
    renderDialog({ entity_ids: [2, 3] })
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Remove Bran" }))
    await user.selectOptions(screen.getByLabelText("Linked entities"), "2")
    await user.click(screen.getByRole("button", { name: "Add" }))
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(patchMutate).not.toHaveBeenCalled()
  })

  it("clears all linked entities", async () => {
    renderDialog()
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Remove Bran" }))
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(patchMutate.mock.calls[0][0]).toEqual({
      id: 9,
      patch: { entity_ids: [] },
    })
  })

  it("shows Referenced by only while Goals is on", () => {
    state.goals = true
    const first = render(
      <TimelineEventDialog
        bookId={1}
        event={event}
        chapterIndex={chapterIndex}
        open
        onOpenChange={vi.fn()}
      />,
    )
    expect(screen.getByRole("heading", { name: "Referenced by" })).toBeInTheDocument()
    expect(screen.getByRole("link", { name: "Find the sword" })).toHaveAttribute(
      "href",
      "/books/1/canon/goals?node=4",
    )
    first.unmount()

    state.goals = false
    render(
      <TimelineEventDialog
        bookId={1}
        event={event}
        chapterIndex={chapterIndex}
        open
        onOpenChange={vi.fn()}
      />,
    )
    expect(screen.queryByRole("heading", { name: "Referenced by" })).toBeNull()
  })

  it("requires a title", async () => {
    renderDialog()
    const user = userEvent.setup()
    await user.clear(screen.getByLabelText("Title"))
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled()
  })
  it("creates a child with the prefilled parent and the chosen status", async () => {
    const user = userEvent.setup()
    render(
      <TimelineEventDialog
        bookId={1}
        event={null}
        events={[
          eventWith({ id: 10, title: "Parent candidate", parent_id: null, position: 10 }),
        ]}
        parentId={10}
        chapterIndex={chapterIndex}
        open
        onOpenChange={vi.fn()}
      />,
    )
    expect(
      screen.getByRole("heading", { name: "New timeline event" }),
    ).toBeInTheDocument()
    expect(screen.getByLabelText("Parent")).toHaveValue("10")
    await user.type(screen.getByLabelText("Title"), "Child scene")
    await user.selectOptions(screen.getByLabelText("Status"), "planned")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(createMutate.mock.calls[0][0]).toEqual({
      title: "Child scene",
      description: "",
      story_time: "",
      entity_ids: [],
      importance: 2,
      status: "planned",
      parent_id: 10,
    })
  })

  it("excludes the entry and its subtree from the parent select", () => {
    render(
      <TimelineEventDialog
        bookId={1}
        event={eventWith({ id: 9, title: "Parent", parent_id: null })}
        events={[
          eventWith({ id: 10, title: "Candidate", parent_id: null, position: 1 }),
          eventWith({ id: 11, title: "Child of nine", parent_id: 9, position: 2 }),
          eventWith({ id: 12, title: "Grandchild", parent_id: 11, position: 3 }),
        ]}
        chapterIndex={chapterIndex}
        open
        onOpenChange={vi.fn()}
      />,
    )
    expect(screen.getByRole("option", { name: "Candidate" })).toBeInTheDocument()
    expect(screen.queryByRole("option", { name: "Child of nine" })).toBeNull()
    expect(screen.queryByRole("option", { name: "Grandchild" })).toBeNull()
  })

  it("shows the 409 timeline.status_conflict reason in the dialog", async () => {
    patchMutate.mockImplementationOnce(
      (_variables: unknown, options: { onError?: (error: unknown) => void }) => {
        options.onError?.(
          new ApiError(409, "timeline.status_conflict", "conflict", {
            event_id: 9,
            reason: "planned_descendant",
            conflict: "#12 Child",
          }),
        )
      },
    )
    const user = userEvent.setup()
    renderDialog({ status: "planned" })
    await user.selectOptions(screen.getByLabelText("Status"), "occurred")
    await user.type(screen.getByLabelText("Change note"), "promote")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(screen.getByRole("alert")).toHaveTextContent(
      "This entry still has a planned sub-entry «#12 Child»; it cannot be marked as happened.",
    )
  })

})
