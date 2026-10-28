import "@/i18n"
/** A late archivist success must not label the action that replaced it (R9-04). */
import { act, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import type { NextActionDto } from "@/api/gen/NextActionDto"

const seedMutate = vi.hoisted(() => vi.fn())
vi.mock("@/api/hooks/conception", () => ({
  useSeedCanon: () => ({ mutate: seedMutate, isPending: false }),
}))
vi.mock("@/api/hooks/reviews", () => ({
  useResyncChapter: () => ({ mutate: vi.fn(), isPending: false }),
  useMaintainChapter: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/api/hooks/tracks", () => ({
  useCreateChapter: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))
vi.mock("@/features/workspace/shell", () => ({ useBookShell: () => undefined }))
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() } }))

import { NextActionCard } from "@/features/overview/NextActionCard"

const SEED = {
  kind: "seed_canon",
  chapter: null,
  track_id: null,
} as unknown as NextActionDto
const SYNC = {
  kind: "sync_canon",
  chapter: { id: 5, number: 1, title: "One", status: "official", canon_status: "stale" },
  track_id: 10,
} as unknown as NextActionDto

describe("NextActionCard late archivist response", () => {
  it("hides the started line when the displayed action changed", async () => {
    const user = userEvent.setup()
    const view = render(<NextActionCard bookId={1} status="active" action={SEED} tracks={[]} />)
    await user.click(screen.getByRole("button", { name: /Go/ }))
    const options = seedMutate.mock.calls[0]?.[1] as { onSuccess?: (r: { session_id: number }) => void }
    view.rerender(<NextActionCard bookId={1} status="active" action={SYNC} tracks={[]} />)
    act(() => {
      options.onSuccess?.({ session_id: 42 })
    })
    expect(screen.queryByRole("status")).not.toBeInTheDocument()
  })
})
