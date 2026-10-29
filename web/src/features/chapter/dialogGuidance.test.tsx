/** Guidance syncing and dialog latching across the permanently mounted outline/write dialogs. */
import { render as testingRender, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type React from "react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import { ApiError } from "@/api/client"
import type { BookSettings } from "@/api/gen/BookSettings"

const writeMutate = vi.fn()
const outlineMutate = vi.fn()

vi.mock("@/api/hooks/chapters", () => ({
  useWriteChapter: () => ({ mutate: writeMutate, isPending: false }),
  useWriteOutlineChapter: () => ({ mutate: outlineMutate, isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import { OutlineDialog } from "@/features/chapter/OutlineDialog"
import { WriteDialog } from "@/features/chapter/WriteDialog"
import { TooltipProvider } from "@/components/ui/tooltip"

const SETTINGS: BookSettings = {
  granularity: "paragraph",
  auto_pause: false,
  pause_after_outline: true,
  target_words_min: 3000,
  target_words_max: 4000,
  recent_summary_count: 5,
  recent_chapter_count: 2,
  midterm_window: 10,
  auto_maintenance: true,
  canon_systems: { entities: true, timeline: true, memory: true, foreshadowing: true, goals: true },
}

function render(ui: React.ReactElement) {
  return testingRender(ui)
}

const GUIDANCE_LABEL = "What should this chapter do?"

beforeEach(() => {
  writeMutate.mockReset()
  outlineMutate.mockReset()
})

describe("guidance follows the last seen server value (R2-01)", () => {
  const writeView = (guidance: string, chapterId = 1) => (
    <TooltipProvider>
      <WriteDialog
        chapterId={chapterId}
        bookId={1}
        settings={SETTINGS}
        guidance={guidance}
        open
        onOpenChange={() => {}}
        onStarted={() => {}}
        onWriteOutline={() => {}}
        hasOutline
      />
    </TooltipProvider>
  )
  const outlineView = (guidance: string, chapterId = 1) => (
    <OutlineDialog
      chapterId={chapterId}
      bookId={1}
      settings={SETTINGS}
      guidance={guidance}
      open
      onOpenChange={() => {}}
      onStarted={() => {}}
    />
  )

  it("WriteDialog adopts a changed guidance prop when untouched", () => {
    const { rerender } = render(writeView("旧要求"))
    rerender(writeView("新要求"))
    expect(screen.getByLabelText(GUIDANCE_LABEL)).toHaveValue("新要求")
  })

  it("WriteDialog keeps typing when the guidance prop changes", async () => {
    const user = userEvent.setup()
    const { rerender } = render(writeView("旧要求"))
    await user.clear(screen.getByLabelText(GUIDANCE_LABEL))
    await user.type(screen.getByLabelText(GUIDANCE_LABEL), "作者正在写的要求")
    rerender(writeView("服务器的新要求"))
    expect(screen.getByLabelText(GUIDANCE_LABEL)).toHaveValue("作者正在写的要求")
  })

  it("OutlineDialog adopts a changed guidance prop when untouched", () => {
    const { rerender } = render(outlineView("旧要求"))
    rerender(outlineView("新要求"))
    expect(screen.getByLabelText(GUIDANCE_LABEL)).toHaveValue("新要求")
  })

  it("OutlineDialog keeps typing when the guidance prop changes", async () => {
    const user = userEvent.setup()
    const { rerender } = render(outlineView("旧要求"))
    await user.clear(screen.getByLabelText(GUIDANCE_LABEL))
    await user.type(screen.getByLabelText(GUIDANCE_LABEL), "作者正在写的细纲要求")
    rerender(outlineView("服务器的新要求"))
    expect(screen.getByLabelText(GUIDANCE_LABEL)).toHaveValue("作者正在写的细纲要求")
  })

  it("replaces a typed WriteDialog draft with chapter B's guidance", async () => {
    const user = userEvent.setup()
    const { rerender } = render(writeView("第一章的要求"))
    await user.clear(screen.getByLabelText(GUIDANCE_LABEL))
    await user.type(screen.getByLabelText(GUIDANCE_LABEL), "我在第一章输入的要求")
    rerender(writeView("第二章的要求", 2))
    expect(screen.getByLabelText(GUIDANCE_LABEL)).toHaveValue("第二章的要求")
  })

  it("replaces a typed OutlineDialog draft with chapter B's guidance", async () => {
    const user = userEvent.setup()
    const { rerender } = render(outlineView("第一章的要求"))
    await user.clear(screen.getByLabelText(GUIDANCE_LABEL))
    await user.type(screen.getByLabelText(GUIDANCE_LABEL), "我在第一章输入的细纲要求")
    rerender(outlineView("第二章的要求", 2))
    expect(screen.getByLabelText(GUIDANCE_LABEL)).toHaveValue("第二章的要求")
  })

  it("resets WriteDialog's draft and outline warning when the chapter changes", async () => {
    const user = userEvent.setup()
    writeMutate.mockImplementation(
      (_input: unknown, options: { onError: (error: unknown) => void }) => {
        options.onError(new ApiError(409, "chapter.outline_missing", "missing"))
      },
    )
    const { rerender } = render(writeView("第一章的要求"))
    await user.click(screen.getByRole("button", { name: "Start writing" }))
    expect(screen.getByText("This chapter has no outline")).toBeInTheDocument()
    rerender(writeView("第二章的要求", 2))
    expect(screen.getByLabelText(GUIDANCE_LABEL)).toHaveValue("第二章的要求")
    expect(screen.queryByText("This chapter has no outline")).not.toBeInTheDocument()
  })

  it("resets OutlineDialog's previous-not-ready warning when the chapter changes", async () => {
    const user = userEvent.setup()
    outlineMutate.mockImplementation(
      (_input: unknown, options: { onError: (error: unknown) => void }) => {
        options.onError(
          new ApiError(409, "chapter.previous_not_ready", "not ready", {
            chapter_number: 1,
            reason: "not_official",
          }),
        )
      },
    )
    const { rerender } = render(outlineView("", 1))
    await user.click(screen.getByRole("button", { name: "Start planning" }))
    expect(screen.getByText("The previous chapter is not ready")).toBeInTheDocument()
    rerender(outlineView("", 2))
    expect(screen.queryByText("The previous chapter is not ready")).not.toBeInTheDocument()
  })

  it("uses the writer placeholder in WriteDialog and the planner placeholder in OutlineDialog", () => {
    const { rerender } = render(writeView(""))
    expect(
      screen.getByPlaceholderText("Optional. The writer reads this before drafting the chapter."),
    ).toBeInTheDocument()
    rerender(outlineView(""))
    expect(
      screen.getByPlaceholderText("Optional. The planner reads this before drafting the outline."),
    ).toBeInTheDocument()
  })
})

describe("OutlineDialog previous_not_ready latch (R2-02)", () => {
  it("clears the warning when the dialog is reopened", async () => {
    const user = userEvent.setup()
    outlineMutate.mockImplementation(
      (_input: unknown, options: { onError: (error: unknown) => void }) => {
        options.onError(
          new ApiError(409, "chapter.previous_not_ready", "not ready", {
            chapter_number: 1,
            reason: "not_official",
          }),
        )
      },
    )
    const view = (open: boolean) => (
      <OutlineDialog
        chapterId={1}
        bookId={1}
        settings={SETTINGS}
        guidance=""
        open={open}
        onOpenChange={() => {}}
        onStarted={() => {}}
      />
    )
    const { rerender } = render(view(true))
    await user.click(screen.getByRole("button", { name: "Start planning" }))
    expect(screen.getByText("The previous chapter is not ready")).toBeInTheDocument()
    rerender(view(false))
    rerender(view(true))
    expect(screen.queryByText("The previous chapter is not ready")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Start planning" })).toBeInTheDocument()
  })
})
