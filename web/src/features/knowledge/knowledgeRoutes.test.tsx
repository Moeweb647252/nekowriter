/**
 * Deep-linkable knowledge routes (spec/11 §3, PLAN 11 hardening task-23).
 *
 * `/knowledge/scopes/:scopeId` selects a scope and
 * `/knowledge/scopes/:scopeId/entries/:entryId` selects a scope and an entry; reloading either URL
 * restores the same view, and a deleted scope or entry renders a not-found state instead of
 * crashing or silently showing something else.
 */
import { render, screen } from "@testing-library/react"
import i18n from "i18next"
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import enKnowledge from "@/i18n/locales/en/knowledge.json"

import KnowledgePage from "./KnowledgePage"

beforeAll(() => {
  if (!i18n.hasResourceBundle("en", "knowledge")) {
    i18n.addResourceBundle("en", "knowledge", enKnowledge)
  }
  if (!i18n.options.ns?.includes("knowledge")) {
    i18n.options.ns = [...(i18n.options.ns as string[]), "knowledge"]
  }
})

const ENTRY = {
  id: 7,
  scope_id: 1,
  title: "Tea houses",
  content: "Song tea houses served tea all day.",
  source: "manual",
  source_urls: [],
  tags: [],
  research_task_id: null,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
}

const state = vi.hoisted(() => ({
  params: {} as Record<string, string>,
  entriesScope: vi.fn(),
  entry: { isPending: false, isError: false, data: undefined as unknown },
}))
const navigate = vi.hoisted(() => vi.fn())

vi.mock("react-router", () => ({
  useParams: () => state.params,
  useNavigate: () => navigate,
}))

const noopMutation = { mutate: vi.fn(), isPending: false }

vi.mock("@/api/hooks/knowledge", () => ({
  useScopes: () => ({
    isPending: false,
    data: [
      { id: 1, name: "Song life", description: "", entry_count: 1 },
      { id: 2, name: "Sword lore", description: "", entry_count: 0 },
    ],
  }),
  useEntries: (scopeId: number | undefined, filter: string) => {
    state.entriesScope(scopeId, filter)
    return {
      isPending: false,
      isFetchingNextPage: false,
      hasNextPage: false,
      fetchNextPage: vi.fn(),
      data: {
        pages: [
          {
            items: [
              {
                id: 7,
                title: "Tea houses",
                source: "manual",
                tags: [],
                updated_at: "2026-01-03T00:00:00Z",
              },
            ],
            next_cursor: null,
          },
        ],
        pageParams: [undefined],
      },
    }
  },
  useEntry: () => state.entry,
  useKnowledgeSearch: () => ({ isPending: false, data: [] }),
  useCreateScope: () => noopMutation,
  usePatchScope: () => noopMutation,
  useDeleteScope: () => noopMutation,
  useCreateEntry: () => noopMutation,
  usePatchEntry: () => noopMutation,
  useDeleteEntry: () => noopMutation,
}))

describe("knowledge deep links", () => {
  beforeEach(() => {
    state.params = {}
    state.entry = { isPending: false, isError: false, data: undefined }
    state.entriesScope.mockClear()
    navigate.mockClear()
  })

  it("selects the scope named by the URL", () => {
    state.params = { scopeId: "2" }
    render(<KnowledgePage />)
    expect(state.entriesScope).toHaveBeenCalledWith(2, "")
  })

  it("selects both the scope and the entry of a deep link", () => {
    state.params = { scopeId: "1", entryId: "7" }
    state.entry = { isPending: false, isError: false, data: ENTRY }
    render(<KnowledgePage />)
    expect(state.entriesScope).toHaveBeenCalledWith(1, "")
    expect(screen.getByRole("heading", { name: "Tea houses" })).toBeInTheDocument()
  })

  it("shows a not-found state for a deleted entry", () => {
    state.params = { scopeId: "1", entryId: "99" }
    state.entry = { isPending: false, isError: true, data: undefined }
    render(<KnowledgePage />)
    expect(screen.getByText("This entry no longer exists.")).toBeInTheDocument()
  })

  it("shows a not-found state for a deleted scope", () => {
    state.params = { scopeId: "99" }
    render(<KnowledgePage />)
    expect(screen.getByText("This scope no longer exists.")).toBeInTheDocument()
  })

  it("navigates to the entry's URL when it is clicked", async () => {
    const userEvent = (await import("@testing-library/user-event")).default
    state.params = { scopeId: "1" }
    render(<KnowledgePage />)
    await userEvent.click(screen.getByText("Tea houses"))
    expect(navigate).toHaveBeenCalledWith("/knowledge/scopes/1/entries/7")
  })
})
