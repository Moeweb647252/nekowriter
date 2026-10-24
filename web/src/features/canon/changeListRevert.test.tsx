/**
 * `change.not_latest` links the changes that must be reverted first (spec/06 §3.1).
 *
 * Only the newest non-reverted change of a target may be reverted; the UI must say "revert the
 * later changes first" and link them, not just toast the code.
 */
import { act, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeAll, describe, expect, it, vi } from "vitest"
import i18n from "i18next"

import enBook from "@/i18n/locales/en/book.json"
import enCanon from "@/i18n/locales/en/canon.json"
import { ApiError } from "@/api/client"

const state = vi.hoisted(() => ({
  mutate: vi.fn(),
}))

vi.mock("@/api/hooks/useCanon", () => ({
  useRevertChange: () => ({ isPending: false, mutate: state.mutate }),
}))

import { ChangeList } from "@/features/canon/ChangeList"

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
  i18n.addResourceBundle("en", "book", enBook, true, true)
})

function change(id: number) {
  return {
    id,
    book_id: 1,
    track_id: null,
    chapter_id: null,
    session_id: null,
    actor: "user" as const,
    target_type: "entity" as const,
    target_id: 7,
    op: "update" as const,
    before: null,
    after: null,
    note: "",
    reverts_id: null,
    reverted_by_id: null,
    created_at: "2026-01-01T00:00:00Z",
  }
}

describe("ChangeList", () => {
  it("links the later changes blocking a revert", async () => {
    const user = userEvent.setup()
    render(<ChangeList bookId={1} changes={[change(12), change(10)]} />)
    await user.click(screen.getAllByRole("button", { name: "Revert" })[1])
    const options = state.mutate.mock.calls[0]?.[1] as {
      onError: (error: unknown) => void
    }
    act(() => {
      options.onError(
        new ApiError(409, "change.not_latest", "a later change exists", {
          later_change_ids: [12],
        }),
      )
    })
    expect(
      screen.getByText(/Revert the later changes first/),
    ).toBeInTheDocument()
    expect(screen.getByRole("link", { name: "#12" })).toHaveAttribute(
      "href",
      "#change-12",
    )
    // The row it links is an anchor on the page.
    expect(document.getElementById("change-12")).not.toBeNull()
  })
})
