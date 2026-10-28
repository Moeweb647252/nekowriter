/** The send-back dialog explains `chapter.outline_missing` and offers Write the outline (D-146/F8). */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

import { ApiError } from "@/api/client"

const sendBackMutate = vi.fn()

vi.mock("@/api/hooks/reviews", () => ({
  useSendBack: () => ({ mutate: sendBackMutate, isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import { SendBackDialog } from "@/features/chapter/SendBackDialog"

beforeEach(() => {
  sendBackMutate.mockReset()
})

describe("SendBackDialog outline_missing", () => {
  it("explains the missing outline and offers Write the outline", async () => {
    const user = userEvent.setup()
    const onWriteOutline = vi.fn()
    sendBackMutate.mockImplementation(
      (_input: unknown, options: { onError: (error: unknown) => void }) => {
        options.onError(new ApiError(409, "chapter.outline_missing", "missing"))
      },
    )
    render(
      <SendBackDialog
        chapterId={1}
        bookId={1}
        open
        onOpenChange={() => {}}
        onWriteOutline={onWriteOutline}
      />,
    )
    await user.type(screen.getByLabelText("Feedback for the writer"), "重写。")
    await user.click(screen.getByRole("button", { name: "Send back" }))
    expect(screen.getByText("This chapter has no outline")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Write the outline" }))
    expect(onWriteOutline).toHaveBeenCalledTimes(1)
  })
})
