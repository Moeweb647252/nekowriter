/**
 * The Memory page carries the facts and world rules table below the three columns
 * (PROJECT §8.8: "Below: the facts and world rules table").
 */
import { render, screen } from "@testing-library/react"
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest"
import i18n from "i18next"

import enCanon from "@/i18n/locales/en/canon.json"

const emptySnapshot = {
  chapter: {
    id: 13,
    number: 13,
    label: "Chapter 13",
    title: "Chapter 13",
    guidance: "",
    outline: "",
    target_min: 3000,
    target_max: 4000,
    has_outline: false,
  },
  is_first: true,
  window_mid: 10,
  recent_summaries: [],
  previous_tail: null,
  protagonist: null,
  recent_changes: [],
  appearances: [],
  absent_important: [],
  key_notes: [],
  open_foreshadowing: [],
  cast: [],
  counts: {
    character: 0,
    organization: 0,
    location: 0,
    item: 0,
    other: 0,
    event: 0,
    fact: 0,
    world_rule: 0,
    note_other: 0,
    foreshadowing_open: 0,
  },
  anchor: null,
}

vi.mock("react-router", () => ({
  useParams: () => ({ bookId: "1" }),
}))

/** The recent-chapter rows the endpoint returns; a test sets them to render the list. */
const createNote = vi.fn()
const patchNote = vi.fn()
let systems = {
  entities: true,
  timeline: true,
  memory: true,
  foreshadowing: true,
  goals: true,
}
let memoryRecent: Array<{
  chapter_id: number
  number: number
  label: string
  title: string
  characters: number
  partial: boolean
}> = []

vi.mock("@/api/hooks/useCanon", () => ({
  useMemory: () => ({
    isPending: false,
    data: {
      snapshot: emptySnapshot,
      rendered: "rendered memory",
      recent_chapters: memoryRecent,
      systems,
    },
  }),
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
  useCreateMemoryNote: () => ({ mutate: createNote }),
  usePatchMemoryNote: () => ({ mutate: patchNote }),
  useDeleteMemoryNote: () => ({ mutate: vi.fn() }),
}))

vi.mock("@/api/hooks/tracks", () => ({
  useTracks: () => ({ data: [{ id: 1, kind: "main", title: "" }] }),
  useChapters: () => ({ data: [] }),
}))

import MemoryPage from "@/features/canon/MemoryPage"

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

beforeEach(() => {
  memoryRecent = []
  systems = { entities: true, timeline: true, memory: true, foreshadowing: true, goals: true }
  emptySnapshot.counts.fact = 1
  emptySnapshot.counts.world_rule = 1
  emptySnapshot.counts.event = 1
  emptySnapshot.counts.foreshadowing_open = 1
  createNote.mockClear()
  patchNote.mockClear()
})

describe("MemoryPage", () => {
  it("shows the facts and world rules table below the columns", () => {
    render(<MemoryPage />)
    expect(screen.getByText("The sword was a gift.")).toBeInTheDocument()
    expect(
      screen.getByRole("heading", { name: "Memory notes" }),
    ).toBeInTheDocument()
  })

  it("names the chapters injected in full", () => {
    memoryRecent = [
      {
        chapter_id: 12,
        number: 12,
        label: "Chapter 12",
        title: "Prev",
        characters: 500,
        partial: false,
      },
    ]
    render(<MemoryPage />)

    expect(screen.getByText("Recent chapters injected in full")).toBeInTheDocument()
    expect(screen.getByText("Chapter 12《Prev》（500 characters）")).toBeInTheDocument()
  })
})


describe("MemoryPage system sections", () => {
  it("keeps the notes editor and data available while Memory is off", async () => {
    systems = { entities: true, timeline: true, memory: false, foreshadowing: true, goals: true }
    render(<MemoryPage />)
    expect(screen.getByRole("heading", { name: "Memory notes" })).toBeInTheDocument()
    expect(screen.getByText("The sword was a gift.")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Add note" })).toBeInTheDocument()
    const input = screen.getByPlaceholderText("What must stay true?")
    const user = (await import("@testing-library/user-event")).default.setup()
    await user.type(input, "A new note")
    await user.click(screen.getByRole("button", { name: "Add note" }))
    expect(createNote).toHaveBeenCalledWith(
      expect.objectContaining({ content: "A new note", kind: "fact" }),
      expect.anything(),
    )
    await user.click(screen.getByRole("button", { name: "Retire" }))
    expect(patchNote).toHaveBeenCalledWith(
      { id: 5, patch: { status: "retired" } },
      expect.anything(),
    )
  })

  it("renders each archive-count fragment under its own system flag", () => {
    systems = { entities: false, timeline: true, memory: true, foreshadowing: true, goals: true }
    render(<MemoryPage />)
    expect(screen.getByText(/1 timeline event/)).toBeInTheDocument()
    expect(screen.getByText(/1 facts, 1 world rules/)).toBeInTheDocument()
    expect(screen.getByText(/1 open foreshadowing/)).toBeInTheDocument()
    expect(screen.queryByText(/1 characters/)).toBeNull()
  })

  it("renders one empty state when every source system is off", () => {
    systems = { entities: false, timeline: false, memory: false, foreshadowing: false, goals: true }
    render(<MemoryPage />)
    expect(screen.getByText("No memory systems are on")).toBeInTheDocument()
    expect(screen.queryByText("Short-term")).toBeNull()
    expect(screen.queryByText("Long-term index")).toBeNull()
    expect(screen.getByRole("heading", { name: "Memory notes" })).toBeInTheDocument()
  })
})
