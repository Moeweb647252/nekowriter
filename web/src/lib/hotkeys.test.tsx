/**
 * The keyboard shortcut helpers (spec/11 §7, PROJECT §8.13): a shortcut must match the exact
 * modifiers, and it must never fire while an IME composition is active — Enter and letters belong
 * to the input method then (AGENTS.md §6).
 */
import { render } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import { isHotkey, useHotkey } from "@/lib/hotkeys"

/** A keydown event; jsdom does not honor `isComposing` in the init dict, so it is defined on it. */
function keydown(init: KeyboardEventInit & { composing?: boolean }): KeyboardEvent {
  const { composing, ...rest } = init
  const event = new KeyboardEvent("keydown", { ...rest, cancelable: true })
  if (composing === true) {
    Object.defineProperty(event, "isComposing", { value: true })
  }
  return event
}

describe("isHotkey", () => {
  it("matches the declared modifiers exactly", () => {
    const quickSwitch = { key: "k", primary: true }
    expect(isHotkey(keydown({ key: "k", ctrlKey: true }), quickSwitch)).toBe(true)
    expect(isHotkey(keydown({ key: "K", metaKey: true }), quickSwitch)).toBe(true)
    expect(isHotkey(keydown({ key: "k" }), quickSwitch)).toBe(false)
    expect(isHotkey(keydown({ key: "j", ctrlKey: true }), quickSwitch)).toBe(false)
    expect(isHotkey(keydown({ key: "k", ctrlKey: true, altKey: true }), quickSwitch)).toBe(false)
  })

  it("matches Alt+P for pause", () => {
    const pause = { key: "p", alt: true }
    expect(isHotkey(keydown({ key: "p", altKey: true }), pause)).toBe(true)
    expect(isHotkey(keydown({ key: "p", altKey: true, ctrlKey: true }), pause)).toBe(false)
  })

  it("ignores every key while an IME composition is active", () => {
    expect(
      isHotkey(keydown({ key: "k", ctrlKey: true, composing: true }), { key: "k", primary: true }),
    ).toBe(false)
    expect(isHotkey(keydown({ key: "p", altKey: true, composing: true }), { key: "p", alt: true })).toBe(
      false,
    )
  })
})

function ShortcutTarget({ onFire }: { onFire: () => void }) {
  useHotkey({ key: "k", primary: true }, onFire)
  return <input aria-label="editor" />
}

describe("useHotkey", () => {
  it("fires for the shortcut and ignores a composition", () => {
    const onFire = vi.fn()
    render(<ShortcutTarget onFire={onFire} />)
    window.dispatchEvent(keydown({ key: "k", ctrlKey: true }))
    expect(onFire).toHaveBeenCalled()
    onFire.mockClear()
    window.dispatchEvent(keydown({ key: "k", ctrlKey: true, composing: true }))
    expect(onFire).not.toHaveBeenCalled()
    // A plain "k" (no primary modifier) is not the shortcut.
    window.dispatchEvent(keydown({ key: "k" }))
    expect(onFire).not.toHaveBeenCalled()
  })
})
