import { describe, expect, it } from "vitest"

import { paragraphKeyAction, type ParagraphKeyInput } from "@/lib/paragraphEditing"

const base: ParagraphKeyInput = {
  key: "Enter",
  composing: false,
  caret: 0,
  value: "",
  primaryModifier: false,
  shiftKey: false,
}

describe("paragraphKeyAction", () => {
  it("inserts a paragraph when Enter is pressed at the end of the text", () => {
    expect(paragraphKeyAction({ ...base, caret: 12, value: "twelve chars" })).toBe("insert-after")
    expect(paragraphKeyAction({ ...base, caret: 0, value: "" })).toBe("insert-after")
  })

  it("keeps Enter inside the text as a line break", () => {
    expect(paragraphKeyAction({ ...base, caret: 3, value: "twelve chars" })).toBe("none")
  })

  it("saves on the primary modifier plus Enter wherever the caret is", () => {
    expect(
      paragraphKeyAction({ ...base, caret: 3, value: "twelve chars", primaryModifier: true }),
    ).toBe("save")
  })

  it("deletes the paragraph on Backspace in an empty one", () => {
    expect(paragraphKeyAction({ ...base, key: "Backspace", value: "" })).toBe("delete-paragraph")
    expect(paragraphKeyAction({ ...base, key: "Backspace", value: "x", caret: 1 })).toBe("none")
  })

  it("cancels on Escape", () => {
    expect(paragraphKeyAction({ ...base, key: "Escape", value: "abc", caret: 1 })).toBe("cancel")
  })

  it("ignores every key while an IME composition is active", () => {
    // Enter and Backspace belong to the IME while composing (AGENTS.md §6).
    for (const key of ["Enter", "Backspace", "Escape"]) {
      expect(
        paragraphKeyAction({ ...base, key, composing: true, caret: 0, value: "" }),
        key,
      ).toBe("none")
      expect(
        paragraphKeyAction({
          ...base,
          key,
          composing: true,
          caret: 12,
          value: "twelve chars",
          primaryModifier: true,
        }),
        `${key} with modifier`,
      ).toBe("none")
    }
  })

  it("leaves shift+Enter to the textarea", () => {
    expect(paragraphKeyAction({ ...base, caret: 12, value: "twelve chars", shiftKey: true })).toBe(
      "none",
    )
  })
})
