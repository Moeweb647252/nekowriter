/**
 * The Changes page filters by session too (PROJECT §8.8: "a filterable log (chapter, session,
 * target type, actor)").
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeAll, describe, expect, it, vi } from "vitest"
import i18n from "i18next"

import enAgent from "@/i18n/locales/en/agent.json"
import enBook from "@/i18n/locales/en/book.json"
import enCanon from "@/i18n/locales/en/canon.json"

const state = vi.hoisted(() => ({
  params: {} as Record<string, unknown>,
}))

vi.mock("react-router", () => ({
  useParams: () => ({ bookId: "1" }),
}))

vi.mock("@/api/hooks/useCanon", () => ({
  useChanges: (_bookId: number, params: Record<string, unknown>) => {
    state.params = params
    return { isPending: false, data: { items: [], next_cursor: null } }
  },
}))

vi.mock("@/api/hooks/sessions", () => ({
  useBookSessions: () => ({
    data: [
      {
        id: 4,
        book_id: 1,
        role: "writer",
        kind: "chapter_write",
        chapter_id: 3,
        track_id: 1,
        research_task_id: null,
        chapter_number: 1,
        research_topic: null,
        model_label: "Demo",
        context_window: 100000,
        status: "completed",
        phase: "drafting",
        granularity: "paragraph",
        auto_pause: false,
        pause_after_outline: true,
        pause_requested: false,
        held_tool_call_id: null,
        base_content_version: 1,
        error: null,
        usage: {},
        parent_session_id: null,
        child_session_id: null,
        retrying: null,
        warnings: [],
        created_at: "",
        updated_at: "",
        finished_at: null,
      },
    ],
  }),
}))

vi.mock("@/api/hooks/tracks", () => ({
  useTracks: () => ({ data: [{ id: 1, kind: "main", title: "" }] }),
  useChapters: () => ({ data: [] }),
}))

import ChangesPage from "@/features/canon/ChangesPage"

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
  i18n.addResourceBundle("en", "book", enBook, true, true)
  i18n.addResourceBundle("en", "agent", enAgent, true, true)
})

describe("ChangesPage", () => {
  it("filters the log by session", async () => {
    const user = userEvent.setup()
    render(<ChangesPage />)
    const option = screen.getByRole("option", { name: /Writing/ })
    await user.selectOptions(screen.getByLabelText("Session"), option)
    expect(state.params.sessionId).toBe(4)
  })
})
