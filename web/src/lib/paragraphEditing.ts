/**
 * Keyboard rules of the paragraph editor (spec/11 §5.2).
 *
 * Two keys change the document structure rather than its text:
 * - **Enter at the end of a paragraph** inserts a new paragraph after it, so the author can keep
 *   writing without reaching for the full-text editor.
 * - **Backspace in an empty paragraph** deletes that paragraph.
 *
 * Both are ignored while an IME composition is active: Enter confirms Chinese and Japanese input,
 * and a composition in progress can leave an empty buffer that Backspace must still reach
 * (AGENTS.md §6, spec/11 §7). Enter at the end cannot be expressed with a bare `Key` check because
 * the caret may sit at the end with a selection, so the rule lives here and is unit-tested.
 */

/** What the editor should do with one keystroke. */
export type ParagraphKeyAction =
  | "none"
  | "insert-after"
  | "delete-paragraph"
  | "save"
  | "cancel"

export type ParagraphKeyInput = {
  key: string
  /** Whether the composition is active; `true` suppresses every structural action. */
  composing: boolean
  /** The caret position, measured in the current value. */
  caret: number
  /** The current value of the editor. */
  value: string
  /** ⌘ or Ctrl. */
  primaryModifier: boolean
  /** Shift. */
  shiftKey: boolean
}

/** Decides what one keystroke means in the paragraph editor. */
export function paragraphKeyAction(input: ParagraphKeyInput): ParagraphKeyAction {
  const { key, composing, caret, value, primaryModifier, shiftKey } = input
  if (composing) {
    return "none"
  }
  if (key === "Escape") {
    return "cancel"
  }
  if (key === "Enter" && primaryModifier) {
    return "save"
  }
  // Enter at the very end of the paragraph starts the next one; Enter inside the text keeps
  // writing a line break within this paragraph, as before.
  if (key === "Enter" && !primaryModifier && !shiftKey && caret === value.length) {
    return "insert-after"
  }
  // Backspace in an empty paragraph removes the paragraph itself (UAX #29 aside, this is the
  // structural edit authors expect from a paragraph list).
  if (key === "Backspace" && value.length === 0) {
    return "delete-paragraph"
  }
  return "none"
}
