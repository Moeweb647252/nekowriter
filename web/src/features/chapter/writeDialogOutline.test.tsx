/** The Write dialog loses pause-after-outline and handles `chapter.outline_missing` (D-146). */
import { render as testingRender, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type React from "react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import { ApiError } from "@/api/client"
import type { BookSettings } from "@/api/gen/BookSettings"

const writeMutate = vi.fn()

vi.mock("@/api/hooks/chapters", () => ({
  useWriteChapter: () => ({ mutate: writeMutate, isPending: false }),
  useWriteOutlineChapter: () => ({ mutate: vi.fn(), isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import { WriteDialog } from "@/features/chapter/WriteDialog"
import { TooltipProvider } from "@/components/ui/tooltip"

function render(ui: React.ReactElement) {
  return testingRender(<TooltipProvider>{ui}</TooltipProvider>)
}

const SETTINGS: BookSettings = {
  granularity: "paragraph",
  auto_pause: false,
  pause_after_outline: true,
  target_words_min: 3000,
  target_words_max: 4000,
  recent_summary_count: 5,
  midterm_window: 10,
  auto_maintenance: true,
}

beforeEach(() => {
  writeMutate.mockReset()
})

describe("WriteDialog", () => {
  it("has no pause-after-outline switch", () => {
    render(
      <WriteDialog
        chapterId={1}
        bookId={1}
        settings={SETTINGS}
        guidance=""
        open
        onOpenChange={() => {}}
        onStarted={() => {}}
        onWriteOutline={() => {}}
      />,
    )
    expect(screen.queryByText("Pause after the outline")).not.toBeInTheDocument()
    // Only auto-pause remains.
    expect(screen.getAllByRole("switch")).toHaveLength(1)
  })

  it("offers Write the outline when the chapter lost its outline", async () => {
    const user = userEvent.setup()
    const onWriteOutline = vi.fn()
    writeMutate.mockImplementation(
      (_input: unknown, options: { onError: (error: unknown) => void }) => {
        options.onError(new ApiError(409, "chapter.outline_missing", "missing"))
      },
    )
    render(
      <WriteDialog
        chapterId={1}
        bookId={1}
        settings={SETTINGS}
        guidance=""
        open
        onOpenChange={() => {}}
        onStarted={() => {}}
        onWriteOutline={onWriteOutline}
      />,
    )
    await user.click(screen.getByRole("button", { name: "Start writing" }))
    expect(screen.getByText("This chapter has no outline")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Write the outline" }))
    expect(onWriteOutline).toHaveBeenCalledTimes(1)
  })
})
