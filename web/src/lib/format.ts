/** Formatting helpers (spec/11 §6). */

import i18n from "@/i18n"

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

/**
 * `"2,350 words"` / `"2,350 字"` (spec/11 §6). The unit comes from the `common:words` resource,
 * never from a literal, so a new locale needs only locale data (AGENTS.md: never concatenate
 * translated fragments). Without an explicit `language` it follows the active i18n language.
 */
export function formatWords(count: number, language = i18n.language): string {
  return i18n.t("common:words", {
    lng: language,
    count: formatNumber(count, language),
  })
}

/** Truncates a string to `max` characters, appending an ellipsis when shortened. */
export function truncate(text: string, max: number): string {
  const characters = Array.from(text)
  if (characters.length <= max) {
    return text
  }
  return `${characters.slice(0, max).join("")}…`
}
