import { describe, expect, it } from "vitest"

import { bucketBoundaries } from "./range"

describe("bucketBoundaries", () => {
  const day = new Date(2026, 9, 14, 15, 30)

  it("splits today into hours and the last days into days, ending at the next midnight", () => {
    const today = bucketBoundaries("today", day)
    expect(today).toHaveLength(25)
    expect(today[0]).toBe(new Date(2026, 9, 14).getTime())
    expect(today.at(-1)).toBe(new Date(2026, 9, 15).getTime())

    const week = bucketBoundaries("week", day)
    expect(week).toHaveLength(8)
    expect(week[0]).toBe(new Date(2026, 9, 8).getTime())
    expect(week.at(-1)).toBe(new Date(2026, 9, 15).getTime())

    expect(bucketBoundaries("month", day)).toHaveLength(31)
  })

  it("always increases", () => {
    for (const range of ["today", "week", "month"] as const) {
      const boundaries = bucketBoundaries(range, day)
      expect(
        boundaries.every((time, index) => index === 0 || time > (boundaries[index - 1] ?? time)),
      ).toBe(true)
    }
  })
})
