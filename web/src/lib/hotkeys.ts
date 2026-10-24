/**
 * Keyboard helpers (spec/11 §7).
 *
 * Enter confirms a Chinese or Japanese IME composition, so shortcuts must ignore keystrokes while
 * a composition is active (AGENTS.md §6). `KeyboardEvent.isComposing` exists at runtime but is not
 * part of React's synthetic event type; this helper reads it through the native event.
 */

/** True while an IME composition is active for this keyboard event. */
export function isComposing(event: { nativeEvent: KeyboardEvent }): boolean {
  const native = event.nativeEvent
  return native.isComposing || native.keyCode === 229
}

/** True when the platform primary modifier is held (⌘ on macOS, Ctrl elsewhere). */
export function isPrimaryModifier(event: { metaKey: boolean; ctrlKey: boolean }): boolean {
  return event.metaKey || event.ctrlKey
}
