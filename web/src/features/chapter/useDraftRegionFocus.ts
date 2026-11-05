import { useEffect, useRef } from "react"

/**
 * Detects leaving the draft region (spec/11 §5.2, D-173 W10/W11).
 *
 * Focus moving from the dock to a paragraph stays: `relatedTarget` is inside the region. A click on
 * unfocusable paragraph text gives a null `relatedTarget`, so the pointer-down target is consulted.
 * The callback is only called while `enabled` and the region really lost focus.
 */
export function useDraftRegionFocus(enabled: boolean, onLeave: () => void) {
  const ref = useRef<HTMLDivElement>(null)
  const pointerTarget = useRef<EventTarget | null>(null)
  const leaveRef = useRef(onLeave)
  useEffect(() => {
    leaveRef.current = onLeave
  }, [onLeave])

  useEffect(() => {
    const node = (ref.current?.closest("[data-draft-region]") ?? ref.current) as
      | HTMLElement
      | null
    if (node === null) return
    const pointerDown = (event: PointerEvent) => {
      pointerTarget.current = event.target
    }
    const focusOut = (event: FocusEvent) => {
      if (!enabled) return
      const next = event.relatedTarget ?? pointerTarget.current
      if (next instanceof Node && node.contains(next)) return
      if (document.hasFocus() === false) return
      leaveRef.current()
    }
    // Capture so the pointer target is recorded before the text editor opens and takes focus.
    document.addEventListener("pointerdown", pointerDown, true)
    node.addEventListener("focusout", focusOut as EventListener)
    return () => {
      document.removeEventListener("pointerdown", pointerDown, true)
      node.removeEventListener("focusout", focusOut as EventListener)
    }
  }, [enabled])

  return ref
}
