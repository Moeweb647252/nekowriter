/**
 * The knowledge entry column (PROJECT §8.10: "the entry (view or edit; sources as links; tags)").
 *
 * The three columns are driven by TanStack Query hooks, so the test mocks the hooks the way the
 * other page tests do (no request layer, no network).
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, describe, expect, it, vi } from "vitest"

import enKnowledge from "@/i18n/locales/en/knowledge.json"

import KnowledgePage from "./KnowledgePage"

// The shared test setup registers the namespaces the M0-M6 pages use. This page also needs
// `knowledge`, and registering it here keeps the shared file untouched.
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
  source_urls: ["https://example.com/song-tea"],
  tags: ["song"],
  research_task_id: null,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
}

const noopMutation = { mutate: vi.fn(), isPending: false }

vi.mock("@/api/hooks/knowledge", () => ({
  useScopes: () => ({
    isPending: false,
    data: [{ id: 1, name: "Song life", description: "Song daily life", entry_count: 1 }],
  }),
  useEntries: () => ({
    isPending: false,
    data: { items: [{ id: 7, title: "Tea houses", source: "manual", tags: ["song"] }], next_cursor: null },
  }),
  useEntry: () => ({ isPending: false, data: ENTRY }),
  useKnowledgeSearch: () => ({ isPending: false, data: [] }),
  useCreateScope: () => noopMutation,
  usePatchScope: () => noopMutation,
  useDeleteScope: () => noopMutation,
  useCreateEntry: () => noopMutation,
  usePatchEntry: () => noopMutation,
  useDeleteEntry: () => noopMutation,
}))

vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

describe("KnowledgePage entry column", () => {
  it("reads the entry with its sources as links and its tags", async () => {
    render(<KnowledgePage />)

    // Reading is the default: the content is prose, not a textarea.
    expect(await screen.findByRole("heading", { name: "Tea houses" })).toBeInTheDocument()
    expect(screen.queryByRole("textbox", { name: "Content" })).not.toBeInTheDocument()

    // PROJECT §8.10: sources are links.
    const link = screen.getByRole("link", { name: "https://example.com/song-tea" })
    expect(link).toHaveAttribute("href", "https://example.com/song-tea")
    expect(link).toHaveAttribute("target", "_blank")
    expect(link).toHaveAttribute("rel", "noreferrer")

    // PROJECT §8.10: tags are shown.
    expect(screen.getByText("song")).toBeInTheDocument()
  })

  it("switches to the editor and back", async () => {
    render(<KnowledgePage />)

    await userEvent.click(await screen.findByRole("button", { name: "Edit entry" }))
    const content = await screen.findByRole("textbox", { name: "Content" })
    expect(content).toHaveValue(ENTRY.content)

    await userEvent.click(screen.getByRole("button", { name: "Cancel" }))
    expect(screen.queryByRole("textbox", { name: "Content" })).not.toBeInTheDocument()
    expect(
      screen.getByRole("link", { name: "https://example.com/song-tea" }),
    ).toBeInTheDocument()
  })
})
