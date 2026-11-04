import { describe, expect, it } from "vitest"

import { layoutTreemap } from "@/lib/treemap"

const bounds = { x: 0, y: 0, width: 600, height: 400 }

describe("layoutTreemap", () => {
  it("keeps areas proportional and cells inside the bounds without overlap", () => {
    const items = [
      { key: "a", weight: 6 },
      { key: "b", weight: 6 },
      { key: "c", weight: 4 },
      { key: "d", weight: 3 },
      { key: "e", weight: 2 },
      { key: "f", weight: 2 },
      { key: "g", weight: 1 },
    ]
    const rects = layoutTreemap(items, bounds)
    expect(rects).toHaveLength(items.length)
    const total = items.reduce((sum, item) => sum + item.weight, 0)
    for (const rect of rects) {
      expect(rect.x).toBeGreaterThanOrEqual(bounds.x)
      expect(rect.y).toBeGreaterThanOrEqual(bounds.y)
      expect(rect.x + rect.width).toBeLessThanOrEqual(bounds.x + bounds.width + 1e-9)
      expect(rect.y + rect.height).toBeLessThanOrEqual(bounds.y + bounds.height + 1e-9)
      const item = items.find((candidate) => candidate.key === rect.key)!
      expect((rect.width * rect.height) / (bounds.width * bounds.height)).toBeCloseTo(item.weight / total, 5)
    }
    const totalArea = rects.reduce((sum, rect) => sum + rect.width * rect.height, 0)
    expect(totalArea).toBeCloseTo(bounds.width * bounds.height, 5)
    for (let left = 0; left < rects.length; left += 1) {
      for (let right = left + 1; right < rects.length; right += 1) {
        const a = rects[left]!
        const b = rects[right]!
        const overlaps =
          a.x < b.x + b.width &&
          b.x < a.x + a.width &&
          a.y < b.y + b.height &&
          b.y < a.y + a.height
        expect(overlaps, `${a.key} overlaps ${b.key}`).toBe(false)
      }
    }
  })

  it("is deterministic", () => {
    const items = [
      { key: "one", weight: 3 },
      { key: "two", weight: 1 },
      { key: "three", weight: 2 },
    ]
    expect(layoutTreemap(items, bounds)).toEqual(layoutTreemap([...items], bounds))
  })

  it("skips zero weights", () => {
    const rects = layoutTreemap([{ key: "zero", weight: 0 }, { key: "one", weight: 1 }], bounds)
    expect(rects.map((rect) => rect.key)).toEqual(["one"])
  })
})
