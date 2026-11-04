/** Pure tree helpers for the timeline page (spec/11 §5.5, D-170). */
import type { TimelineEventDto } from "@/api/gen/TimelineEventDto"

/** Every descendant of one entry in the flat story-order list. */
export function descendantsOf(
  id: number,
  events: TimelineEventDto[],
): TimelineEventDto[] {
  const found: TimelineEventDto[] = []
  let growing = true
  while (growing) {
    growing = false
    for (const row of events) {
      if (
        row.parent_id !== null &&
        (row.parent_id === id || found.some((parent) => parent.id === row.parent_id)) &&
        !found.some((seen) => seen.id === row.id)
      ) {
        found.push(row)
        growing = true
      }
    }
  }
  return found
}

/**
 * The move payload for a drag ending on a sibling. It only ever carries the new neighbours, so a
 * drop can never change the parent (D-170: the dialog owns parent changes).
 */
export function siblingMove(
  active: TimelineEventDto,
  over: TimelineEventDto,
  siblings: TimelineEventDto[],
): { afterId?: number; beforeId?: number } | null {
  if (active.parent_id !== over.parent_id) {
    return null
  }
  const from = siblings.findIndex((row) => row.id === active.id)
  const to = siblings.findIndex((row) => row.id === over.id)
  if (from < 0 || to < 0 || from === to) {
    return null
  }
  const target = siblings[to]
  return from < to ? { afterId: target.id } : { beforeId: target.id }
}
