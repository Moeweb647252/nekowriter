/** Formatting helpers (spec/11 §6). */

export function formatNumber(value: number, language = "en"): string {
  return new Intl.NumberFormat(language).format(value)
}

export function formatDateTime(value: string, language = "en"): string {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) {
    return value
  }
  return new Intl.DateTimeFormat(language, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date)
}

export function formatRelativeTime(value: string, language = "en"): string {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) {
    return value
  }
  const formatter = new Intl.RelativeTimeFormat(language, { numeric: "auto" })
  const seconds = Math.round((date.getTime() - Date.now()) / 1000)
  const steps: [Intl.RelativeTimeFormatUnit, number][] = [
    ["second", 60],
    ["minute", 60],
    ["hour", 24],
    ["day", 7],
    ["week", 4.34],
    ["month", 12],
    ["year", Number.POSITIVE_INFINITY],
  ]
  let value0 = seconds
  for (const [unit, size] of steps) {
    if (Math.abs(value0) < size) {
      return formatter.format(Math.round(value0), unit)
    }
    value0 /= size
  }
  return formatter.format(Math.round(value0), "year")
}

export function formatWords(count: number, language = "en"): string {
  const formatted = formatNumber(count, language)
  return language.startsWith("zh") ? `${formatted} 字` : `${formatted} words`
}

/** Truncates a string to `max` characters, appending an ellipsis when shortened. */
export function truncate(text: string, max: number): string {
  const characters = Array.from(text)
  if (characters.length <= max) {
    return text
  }
  return `${characters.slice(0, max).join("")}…`
}
