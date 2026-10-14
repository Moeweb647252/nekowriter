/** The time ranges the dashboard offers (§15). */
export type DashboardRange = "today" | "week" | "month"

export const RANGES: readonly DashboardRange[] = ["today", "week", "month"]

/**
 * The boundaries of the range's buckets as UTC milliseconds, taken in the browser's time
 * zone: today by the hour, the last 7 or 30 days by the day, ending at the next midnight.
 * An hour that a daylight-saving change skips gets no bucket of its own.
 */
export function bucketBoundaries(range: DashboardRange, day: Date): number[] {
  const [year, month, date] = [day.getFullYear(), day.getMonth(), day.getDate()]
  const times =
    range === "today"
      ? Array.from({ length: 25 }, (_, hour) => new Date(year, month, date, hour).getTime())
      : Array.from({ length: (range === "week" ? 7 : 30) + 1 }, (_, index) =>
          new Date(year, month, date - (range === "week" ? 6 : 29) + index).getTime(),
        )
  return times.filter((time, index) => index === 0 || time > (times[index - 1] ?? time))
}
