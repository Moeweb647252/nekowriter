/**
 * Pure squarified treemap layout (M23, plan §3.4).
 *
 * The implementation follows the standard squarify row heuristic: items are placed in a
 * shortest-side strip, adding an item while the worst aspect ratio improves. It has no React,
 * DOM, or time dependency, so a snapshot is stable across renders and test environments.
 */

export type TreemapInput = {
  key: string
  weight: number
}

export type TreemapRect = {
  key: string
  x: number
  y: number
  width: number
  height: number
}

export type TreemapBounds = {
  x: number
  y: number
  width: number
  height: number
}

type MutableRect = TreemapRect

function area(rect: TreemapBounds): number {
  return Math.max(0, rect.width) * Math.max(0, rect.height)
}

function worst(row: TreemapInput[], side: number, scale: number): number {
  if (row.length === 0 || side <= 0) return Number.POSITIVE_INFINITY
  const areas = row.map((item) => item.weight * scale)
  const sum = areas.reduce((total, value) => total + value, 0)
  const max = Math.max(...areas)
  const min = Math.min(...areas)
  if (sum <= 0 || min <= 0) return Number.POSITIVE_INFINITY
  return Math.max((side * side * max) / (sum * sum), (sum * sum) / (side * side * min))
}

function placeRow(
  row: TreemapInput[],
  bounds: TreemapBounds,
  scale: number,
): TreemapRect[] {
  const total = row.reduce((sum, item) => sum + item.weight * scale, 0)
  const horizontal = bounds.width >= bounds.height
  const side = horizontal ? bounds.height : bounds.width
  const thickness = total / Math.max(side, 1e-9)
  const result: TreemapRect[] = []
  let offset = 0
  for (const item of row) {
    const itemArea = item.weight * scale
    const length = itemArea / Math.max(thickness, 1e-9)
    if (horizontal) {
      result.push({
        key: item.key,
        x: bounds.x,
        y: bounds.y + offset,
        width: Math.min(thickness, bounds.width),
        height: Math.min(length, bounds.height),
      })
    } else {
      result.push({
        key: item.key,
        x: bounds.x + offset,
        y: bounds.y,
        width: Math.min(length, bounds.width),
        height: Math.min(thickness, bounds.height),
      })
    }
    offset += length
  }
  return result
}

function squarify(
  items: TreemapInput[],
  bounds: TreemapBounds,
  scale: number,
  out: TreemapRect[],
): void {
  if (items.length === 0 || bounds.width <= 0 || bounds.height <= 0) return
  if (items.length === 1) {
    out.push({ key: items[0]!.key, x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height })
    return
  }
  const side = Math.min(bounds.width, bounds.height)
  const row: TreemapInput[] = []
  let index = 0
  while (index < items.length) {
    const candidate = items[index]!
    const currentWorst = worst(row, side, scale)
    const nextWorst = worst([...row, candidate], side, scale)
    if (row.length === 0 || nextWorst <= currentWorst) {
      row.push(candidate)
      index += 1
    } else {
      break
    }
  }
  const placed = placeRow(row, bounds, scale)
  out.push(...placed)
  const rowArea = row.reduce((sum, item) => sum + item.weight * scale, 0)
  const horizontal = bounds.width >= bounds.height
  const thickness = rowArea / Math.max(side, 1e-9)
  const restBounds: TreemapBounds = horizontal
    ? { x: bounds.x + thickness, y: bounds.y, width: Math.max(0, bounds.width - thickness), height: bounds.height }
    : { x: bounds.x, y: bounds.y + thickness, width: bounds.width, height: Math.max(0, bounds.height - thickness) }
  squarify(items.slice(index), restBounds, scale, out)
}

/** Lay out weighted items in a rectangle. Zero/negative weights are skipped. */
export function layoutTreemap(items: TreemapInput[], bounds: TreemapBounds): TreemapRect[] {
  const usable = items
    .filter((item) => Number.isFinite(item.weight) && item.weight > 0)
    .map((item, index) => ({ ...item, index }))
    .sort((left, right) => right.weight - left.weight || left.index - right.index)
  const total = usable.reduce((sum, item) => sum + item.weight, 0)
  if (total <= 0 || bounds.width <= 0 || bounds.height <= 0) return []
  const scale = area(bounds) / total
  const out: MutableRect[] = []
  squarify(usable, bounds, scale, out)
  return out
}
