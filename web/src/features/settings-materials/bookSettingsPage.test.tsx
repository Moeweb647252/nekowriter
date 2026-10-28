/**
 * Book settings › Model overrides per role (PROJECT §8.12: Book settings includes "model overrides
 * per role"; spec/03 §4 step 1 resolves `book.model_overrides[role]` before the global default).
 *
 * The backend already stored, patched and resolved the mapping; the page never rendered it, so a
 * book's role → model choice could not be made anywhere in the app. These are UI-level tests: the
 * section, the "use the global default" option, the displayed global default, the missing-profile
 * state, and — the classic bug of this feature — the saved object carrying every sibling role.
 */
import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import enBook from "@/i18n/locales/en/book.json"
import enSettings from "@/i18n/locales/en/settings.json"
import type { BookDto } from "@/api/gen/BookDto"

const patchBook = vi.fn()

/** The book the page loads; tests override `model_overrides` / `status` as needed. */
let book: BookDto

/** The global role defaults the page compares each row against. */
let rolesDefaults: Record<string, number | null>

vi.mock("react-router", () => ({
  useParams: () => ({ bookId: "1" }),
}))

vi.mock("@/api/hooks/books", () => ({
  useBook: () => ({ isPending: false, isError: false, data: book }),
  usePatchBook: () => ({ mutate: patchBook, isPending: false }),
}))

vi.mock("@/api/hooks/app-settings", () => ({
  useAppSettings: () => ({
    isPending: false,
    isError: false,
    data: { roles_defaults: rolesDefaults },
  }),
}))

vi.mock("@/api/hooks/providers", () => ({
  useModelProfiles: () => ({
    isPending: false,
    isError: false,
    data: [
      { id: 7, provider_id: 1, display_name: "Writer model" },
      { id: 8, provider_id: 2, display_name: "Review model" },
    ],
  }),
  useProviders: () => ({
    isPending: false,
    isError: false,
    data: [
      { id: 1, name: "Anthropic" },
      { id: 2, name: "DeepSeek" },
    ],
  }),
}))

vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import BookSettingsPage from "./BookSettingsPage"

beforeAll(() => {
  // The shared test setup ships a subset of the namespaces; this page needs `settings` too.
  if (!i18n.hasResourceBundle("en", "book")) {
    i18n.addResourceBundle("en", "book", enBook)
  }
  if (!i18n.hasResourceBundle("en", "settings")) {
    i18n.addResourceBundle("en", "settings", enSettings)
  }
  // jsdom does not implement scrolling; Radix's SelectItem calls it when the list opens.
  Element.prototype.scrollIntoView = vi.fn()
  // Radix's Select trigger guards its pointer-down handler with these jsdom gaps.
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.setPointerCapture = vi.fn()
  Element.prototype.releasePointerCapture = vi.fn()
})

beforeEach(() => {
  patchBook.mockClear()
  rolesDefaults = { conception: 8, researcher: 8, writer: 7, reviewer: 8, archivist: 8 }
  book = {
    id: 1,
    title: "青云劫",
    idea: "",
    genre: "xianxia",
    writing_language: "zh-CN",
    status: "active",
    settings: {
      granularity: "paragraph",
      auto_pause: false,
      pause_after_outline: true,
      target_words_min: 3000,
      target_words_max: 4000,
      recent_summary_count: 5,
      midterm_window: 10,
      auto_maintenance: true,
    },
    model_overrides: { conception: null, researcher: null, writer: null, reviewer: null, archivist: null },
    conception: {
      session_id: null,
      characters_revision: 0,
      characters_approved_revision: null,
      topics_revision: 0,
      topics_approved_revision: null,
      finalized_at: null,
    },
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  }
})

describe("Book settings › model overrides", () => {
  it("renders one row per agent role, all on the global default when nothing is overridden", () => {
    render(<BookSettingsPage />)

    // One row per role, in the order the settings pages use.
    const rows = screen.getAllByTestId(/^model-override-row-/)
    expect(rows.map((row) => row.getAttribute("data-testid"))).toEqual([
      "model-override-row-conception",
      "model-override-row-researcher",
      "model-override-row-planner",
      "model-override-row-writer",
      "model-override-row-reviewer",
      "model-override-row-archivist",
    ])

    // With no override the select shows "Use the global default".
    for (const role of ["conception", "researcher", "planner", "writer", "reviewer", "archivist"]) {
      const row = screen.getByTestId(`model-override-row-${role}`)
      expect(within(row).getByRole("combobox")).toHaveTextContent("Use the global default")
    }
  })

  it("shows the current override as the selected option", () => {
    book.model_overrides = {
      conception: null,
      researcher: null,
      writer: 7,
      reviewer: null,
      archivist: null,
    }
    render(<BookSettingsPage />)

    const writer = screen.getByTestId("model-override-row-writer")
    expect(within(writer).getByRole("combobox")).toHaveTextContent("Anthropic · Writer model")
    const reviewer = screen.getByTestId("model-override-row-reviewer")
    expect(within(reviewer).getByRole("combobox")).toHaveTextContent("Use the global default")
  })

  it("displays what the global default resolves to for every role", () => {
    rolesDefaults = { conception: 8, researcher: null, writer: 7, reviewer: 8, archivist: null }
    render(<BookSettingsPage />)

    expect(screen.getByTestId("model-override-row-conception")).toHaveTextContent(
      "Global default: DeepSeek · Review model",
    )
    expect(screen.getByTestId("model-override-row-writer")).toHaveTextContent(
      "Global default: Anthropic · Writer model",
    )
    // No global default: say so instead of leaving the line empty.
    expect(screen.getByTestId("model-override-row-researcher")).toHaveTextContent(
      "No global default is set for this role.",
    )
    expect(screen.getByTestId("model-override-row-archivist")).toHaveTextContent(
      "No global default is set for this role.",
    )
  })

  it("renders the missing state for an override whose profile was deleted", () => {
    book.model_overrides = {
      conception: null,
      researcher: null,
      writer: 99,
      reviewer: null,
      archivist: null,
    }
    render(<BookSettingsPage />)

    const writer = screen.getByTestId("model-override-row-writer")
    // Not a blank select: the trigger names the missing model and the row explains it.
    expect(within(writer).getByRole("combobox")).toHaveTextContent("Missing model")
    expect(writer).toHaveTextContent(
      "The model set here no longer exists. Pick another model or the global default.",
    )
  })

  it("lets a stale override be cleared back to the global default", async () => {
    // The state a real database reaches after deleting a model profile: the book still names the
    // dead id. The select must show it (not render blank as "no override") and the author must be
    // able to clear it, or the role is stuck on a model that fails with `model.missing_role`.
    const user = userEvent.setup()
    book.model_overrides = {
      conception: null,
      researcher: null,
      writer: 99,
      reviewer: null,
      archivist: null,
    }
    render(<BookSettingsPage />)

    await user.click(
      within(screen.getByTestId("model-override-row-writer")).getByRole("combobox"),
    )
    await user.click(await screen.findByRole("option", { name: "Use the global default" }))
    await user.click(screen.getByRole("button", { name: "Save" }))

    const [body] = patchBook.mock.calls[0] as [{ model_overrides: Record<string, number | null> }]
    expect(body.model_overrides.writer).toBeNull()
  })

  it("saves a chosen model with the whole object, preserving the sibling roles", async () => {
    const user = userEvent.setup()
    book.model_overrides = {
      conception: null,
      researcher: 8,
      writer: null,
      reviewer: 8,
      archivist: null,
    }
    render(<BookSettingsPage />)

    await user.click(
      within(screen.getByTestId("model-override-row-writer")).getByRole("combobox"),
    )
    await user.click(await screen.findByRole("option", { name: "Anthropic · Writer model" }))
    await user.click(screen.getByRole("button", { name: "Save" }))

    expect(patchBook).toHaveBeenCalledTimes(1)
    const [body] = patchBook.mock.calls[0] as [Record<string, unknown>]
    // The full object: `researcher` and `reviewer` keep their overrides instead of being dropped
    // by a patch that only carried the edited role.
    expect(body.model_overrides).toEqual({
      conception: null,
      researcher: 8,
      planner: null,
      writer: 7,
      reviewer: 8,
      archivist: null,
    })
    // And nothing else is invented: the patch carries the form's other fields too, but no second
    // save path exists.
    expect(body).toMatchObject({ title: "青云劫", writing_language: "zh-CN" })
  })

  it("sends null when a role is switched back to the global default", async () => {
    const user = userEvent.setup()
    book.model_overrides = {
      conception: null,
      researcher: null,
      writer: 7,
      reviewer: 8,
      archivist: null,
    }
    render(<BookSettingsPage />)

    await user.click(
      within(screen.getByTestId("model-override-row-writer")).getByRole("combobox"),
    )
    await user.click(await screen.findByRole("option", { name: "Use the global default" }))
    await user.click(screen.getByRole("button", { name: "Save" }))

    const [body] = patchBook.mock.calls[0] as [{ model_overrides: Record<string, number | null> }]
    expect(body.model_overrides.writer).toBeNull()
    // The sibling override survives.
    expect(body.model_overrides.reviewer).toBe(8)
  })

  it("disables the section with a reason while the book is conceiving", () => {
    book.status = "conceiving"
    render(<BookSettingsPage />)

    for (const role of ["conception", "researcher", "planner", "writer", "reviewer", "archivist"]) {
      expect(
        within(screen.getByTestId(`model-override-row-${role}`)).getByRole("combobox"),
      ).toBeDisabled()
    }
    expect(
      screen.getByText(
        "Finalize the conception first: the conception flow chooses the roles until the book is active.",
      ),
    ).toBeInTheDocument()
  })

  it("disables the section with a reason while the book is archived", () => {
    book.status = "archived"
    render(<BookSettingsPage />)

    expect(
      within(screen.getByTestId("model-override-row-writer")).getByRole("combobox"),
    ).toBeDisabled()
    expect(
      screen.getByText("This book is archived. Unarchive it to change its roles."),
    ).toBeInTheDocument()
  })
})
