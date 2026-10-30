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
  useCreateMemoryNote: () => ({ mutate: vi.fn() }),
  usePatchMemoryNote: () => ({ mutate: vi.fn() }),
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
  it("leaves out the entity and memory sections of disabled systems", () => {
    systems = { entities: false, timeline: true, memory: false, foreshadowing: true, goals: true }
    render(<MemoryPage />)
    expect(screen.queryByText(/Mid-term \(last 10 chapters\)/)).toBeNull()
    expect(screen.queryByRole("heading", { name: "Memory notes" })).toBeNull()
    expect(screen.getByText("Short-term")).toBeInTheDocument()
  })
})
