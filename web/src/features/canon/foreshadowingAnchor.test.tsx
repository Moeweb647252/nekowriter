/** Foreshadowing anchors: planned marker and the legacy text-only direction (D-170). */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import enCanon from "@/i18n/locales/en/canon.json"

const patchMutate = vi.hoisted(() => vi.fn())

vi.mock("react-router", async () => {
  const React = await import("react")
  return {
    useParams: () => ({ bookId: "1" }),
    Link: ({ to, children }: { to: string; children: React.ReactNode }) =>
      React.createElement("a", { href: to }, children),
  }
})
vi.mock("@/api/hooks/useCanon", () => ({
  useForeshadowing: () => ({
    isPending: false,
    data: [
      {
        id: 9, book_id: 1, track_id: null, title: "Anchored hook",
        description: "", planned_payoff: "The sword returns.",
        payoff_event_id: 12, payoff_position: "after",
        payoff_event_title: "The gate", payoff_event_status: "planned",
        status: "open", planted_chapter_id: null, resolved_chapter_id: null,
        resolution: "", importance: 3, entity_ids: [], created_at: "", updated_at: "",
      },
      {
        id: 10, book_id: 1, track_id: null, title: "Legacy hook",
        description: "", planned_payoff: "The old direction.",
        payoff_event_id: null, payoff_position: null,
        payoff_event_title: null, payoff_event_status: null,
        status: "open", planted_chapter_id: null, resolved_chapter_id: null,
        resolution: "", importance: 3, entity_ids: [], created_at: "", updated_at: "",
      },
    ],
  }),
  useCreateForeshadowing: () => ({ mutate: vi.fn() }),
  usePatchForeshadowing: () => ({ mutate: patchMutate, isPending: false }),
  useDeleteForeshadowing: () => ({ mutate: vi.fn() }),
  useTimeline: () => ({
    data: [
      { id: 1, book_id: 1, track_id: null, position: 0, story_time: "", title: "", description: "", chapter_id: null, parent_id: null, kind: "root", status: "occurred", child_count: 1, importance: 2, entity_ids: [], created_at: "", updated_at: "" },
      { id: 11, book_id: 1, track_id: null, position: 1024, story_time: "", title: "Dropped gate", description: "", chapter_id: null, parent_id: null, kind: "entry", status: "abandoned", child_count: 0, importance: 2, entity_ids: [], created_at: "", updated_at: "" },
      { id: 12, book_id: 1, track_id: null, position: 2048, story_time: "", title: "The gate", description: "", chapter_id: null, parent_id: null, kind: "entry", status: "planned", child_count: 0, importance: 2, entity_ids: [], created_at: "", updated_at: "" },
      { id: 13, book_id: 1, track_id: null, position: 3072, story_time: "", title: "The battle", description: "", chapter_id: null, parent_id: null, kind: "entry", status: "occurred", child_count: 0, importance: 2, entity_ids: [], created_at: "", updated_at: "" },
    ],
  }),
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

import ForeshadowingPage from "@/features/canon/ForeshadowingPage"

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

beforeEach(() => {
  patchMutate.mockReset()
})

describe("ForeshadowingPage anchors", () => {
  it("shows the anchor, the planned marker and the direction", () => {
    render(<ForeshadowingPage />)
    expect(screen.getByText("Pays off after «The gate»")).toBeInTheDocument()
    expect(screen.getByText("Planned")).toBeInTheDocument()
    expect(screen.getByText(/The sword returns\./)).toBeInTheDocument()
  })

  it("edits the anchor with the timeline picker", async () => {
    const user = userEvent.setup()
    render(<ForeshadowingPage />)
    await user.click(screen.getAllByRole("button", { name: "Edit foreshadowing" })[0])
    expect(
      screen.getByRole("heading", { name: "Edit foreshadowing" }),
    ).toBeInTheDocument()
    const eventSelect = screen.getByLabelText("Timeline event")
    expect(
      Array.from(eventSelect.querySelectorAll("option")).map((option) => option.textContent),
    ).toEqual(["None", "The gate (Planned)", "The battle"])
    await user.selectOptions(eventSelect, "13")
    await user.selectOptions(screen.getByLabelText("Position"), "before")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(patchMutate.mock.calls[0][0]).toEqual({
      id: 9,
      patch: { payoff_event_id: 13, payoff_position: "before" },
    })
  })

  it("clears the anchor as a pair", async () => {
    const user = userEvent.setup()
    render(<ForeshadowingPage />)
    await user.click(screen.getAllByRole("button", { name: "Edit foreshadowing" })[0])
    await user.click(screen.getByRole("button", { name: "Clear anchor" }))
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(patchMutate.mock.calls[0][0]).toEqual({
      id: 9,
      patch: { payoff_event_id: null },
    })
  })

  it("keeps the legacy text-only direction", () => {
    render(<ForeshadowingPage />)
    expect(screen.getByText(/The old direction\./)).toBeInTheDocument()
  })
})
