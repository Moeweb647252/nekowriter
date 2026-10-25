/**
 * Keyboard helpers (spec/11 §7, PROJECT §8.13).
 *
 * Enter confirms a Chinese or Japanese IME composition, so shortcuts must ignore keystrokes while
 * a composition is active (AGENTS.md §6). `KeyboardEvent.isComposing` exists at runtime but is not
 * part of React's synthetic event type; this helper reads it through the native event.
 *
 * The shortcuts themselves are a small custom hook — no library (spec/11 §7):
 * ⌘/Ctrl+K for the quick switch, Alt+P for pause/continue. Contextual ones (⌘/Ctrl+Enter in the
 * composer and editors, Esc in dialogs) are handled by the component that owns the context.
 */
import { useEffect, useRef } from "react"

/** True while an IME composition is active for this keyboard event. */
export function isComposing(event: { nativeEvent: KeyboardEvent }): boolean {
  const native = event.nativeEvent
  return native.isComposing || native.keyCode === 229
}

/** True when the platform primary modifier is held (⌘ on macOS, Ctrl elsewhere). */
export function isPrimaryModifier(event: { metaKey: boolean; ctrlKey: boolean }): boolean {
  return event.metaKey || event.ctrlKey
}

/** A window-level shortcut. Letters are matched case-insensitively. */
export type Hotkey = {
  /** `event.key`, e.g. `"k"`, `"p"`, `"Enter"`, `"Escape"`. */
  key: string
  /** ⌘ on macOS, Ctrl elsewhere. */
  primary?: boolean
  alt?: boolean
  shift?: boolean
}

/**
 * True when this event is the shortcut. An IME composition always wins: the keystroke belongs to
 * the input method, never to a shortcut (AGENTS.md §6).
 */
export function isHotkey(event: KeyboardEvent, hotkey: Hotkey): boolean {
  if (event.isComposing || event.keyCode === 229) {
    return false
  }
  if (event.key.toLowerCase() !== hotkey.key.toLowerCase()) {
    return false
  }
  if ((hotkey.primary ?? false) !== isPrimaryModifier(event)) {
    return false
  }
  if ((hotkey.alt ?? false) !== event.altKey) {
    return false
  }
  if ((hotkey.shift ?? false) !== event.shiftKey) {
    return false
  }
  // A shortcut that takes the primary modifier must not fire while a plain modifier combination
  // would print the letter; the exact modifier check above already guarantees that for the ones
  // declared. Ctrl+Alt is excluded because AltGr reports it on some layouts.
  return true
}

/**
 * Registers one window-level shortcut for as long as the component is mounted, and always calls
 * the latest handler (a re-render does not re-register the listener).
 */
export function useHotkey(hotkey: Hotkey, handler: (event: KeyboardEvent) => void): void {
  const handlerRef = useRef(handler)
  // Keep the latest handler without re-registering the listener. The assignment happens in an
  // effect because writing a ref during render is not allowed under the React Compiler rules.
  useEffect(() => {
    handlerRef.current = handler
  }, [handler])
  const { key, primary, alt, shift } = hotkey
  useEffect(() => {
    const listener = (event: KeyboardEvent) => {
      if (isHotkey(event, { key, primary, alt, shift })) {
        handlerRef.current(event)
      }
    }
    window.addEventListener("keydown", listener)
    return () => window.removeEventListener("keydown", listener)
  }, [key, primary, alt, shift])
}
