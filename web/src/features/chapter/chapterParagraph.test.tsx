/**
 * The reading column's paragraph (spec/11 §5.2, PLAN 11.3): each paragraph is memoized on its own
 * data, so a streaming tail paragraph never re-renders the other hundreds, and the edit callbacks
 * identify the paragraph by pid (stable identities are what makes the memo effective).
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import { Paragraph } from "@/features/chapter/ChapterText"

describe("Paragraph", () => {
  it("is memoized", () => {
    // A bare function would re-render every paragraph on every streamed delta.
    expect((Paragraph as unknown as { $$typeof: symbol }).$$typeof).toBe(Symbol.for("react.memo"))
  })

  it("saves an edited paragraph with its pid", async () => {
    const onSave = vi.fn()
    render(
      <Paragraph
        pid="p3"
        text="第一段。"
        onlyParagraph={false}
        saving={false}
        locked={false}
        onHighlightEnd={vi.fn()}
        onSave={onSave}
        onInsertAfter={vi.fn()}
        onDelete={vi.fn()}
      />,
    )
    await userEvent.click(screen.getByText("第一段。"))
    const editor = screen.getByRole("textbox")
    await userEvent.clear(editor)
    await userEvent.type(editor, "改过的段落。")
    await userEvent.tab()
    expect(onSave).toHaveBeenCalledWith("p3", "改过的段落。")
  })

  it("does not open the editor while the chapter is locked", async () => {
    const onSave = vi.fn()
    render(
      <Paragraph
        pid="p1"
        text="第一段。"
        onlyParagraph
        saving={false}
        locked
        onHighlightEnd={vi.fn()}
        onSave={onSave}
        onInsertAfter={vi.fn()}
        onDelete={vi.fn()}
      />,
    )
    await userEvent.click(screen.getByText("第一段。"))
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument()
  })
})
