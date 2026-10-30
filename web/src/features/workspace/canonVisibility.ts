/** One source of truth for which canon pages a book's switches hide and show (spec/11 §5.1, D-150). */
import type { CanonSystems } from "@/api/gen/CanonSystems"

/** The canon systems that own a navigable page. `goals` is reserved for the Goals page. */
export type CanonPageSystem = "entities" | "timeline" | "memory" | "foreshadowing" | "goals"

/** True when the page of `system` should be shown for this book. */
export function isCanonPageVisible(systems: CanonSystems, system: CanonPageSystem): boolean {
  return systems[system]
}

/** The canon system whose page is open at this path, when it is one. */
export function canonPageSystem(pathname: string): CanonPageSystem | null {
  const match = /\/canon\/(entities|timeline|memory|foreshadowing|goals)(?:\/|$)/.exec(pathname)
  return (match?.[1] as CanonPageSystem | undefined) ?? null
}
