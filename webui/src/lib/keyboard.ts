import type { KeyboardEvent } from "react"

/**
 * Submits the field's form on Ctrl+Enter or Cmd+Enter (§18.5), but never while an input
 * method is composing text.
 */
export function submitOnModEnter(event: KeyboardEvent<HTMLTextAreaElement>) {
  if (event.key !== "Enter" || !(event.ctrlKey || event.metaKey)) return
  if (event.nativeEvent.isComposing) return
  event.preventDefault()
  event.currentTarget.form?.requestSubmit()
}
