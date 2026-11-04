/** Foreshadowing anchors: planned marker and the legacy text-only direction (D-170). */
import { render, screen } from "@testing-library/react"
import i18n from "i18next"
import { beforeAll, describe, expect, it, vi } from "vitest"

import enCanon from "@/i18n/locales/en/canon.json"

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
  usePatchForeshadowing: () => ({ mutate: vi.fn() }),
  useDeleteForeshadowing: () => ({ mutate: vi.fn() }),
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

describe("ForeshadowingPage anchors", () => {
  it("shows the anchor, the planned marker and the direction", () => {
    render(<ForeshadowingPage />)
    expect(
      screen.getByText(/Planned payoff: after «The gate»/),
    ).toBeInTheDocument()
    expect(screen.getByText(/Planned payoff: after «The gate» \(planned\)/)).toBeInTheDocument()
    expect(screen.getByText(/The sword returns\./)).toBeInTheDocument()
  })

  it("keeps the legacy text-only direction", () => {
    render(<ForeshadowingPage />)
    expect(screen.getByText(/The old direction\./)).toBeInTheDocument()
  })
})
