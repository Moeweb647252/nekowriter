import { useMemo } from "react"
import { useTranslation } from "react-i18next"

import type { Timestamp } from "@/api/generated/Timestamp"

/** Number and date formatting in the interface language. */
export function useFormatters() {
  const { i18n } = useTranslation()
  const locale = i18n.resolvedLanguage ?? i18n.language

  return useMemo(() => {
    const dateTime = new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" })
    const day = new Intl.DateTimeFormat(locale, { month: "short", day: "numeric" })
    const hour = new Intl.DateTimeFormat(locale, { hour: "numeric", minute: "2-digit" })
    const number = new Intl.NumberFormat(locale)
    const signed = new Intl.NumberFormat(locale, { signDisplay: "exceptZero" })
    const percent = new Intl.NumberFormat(locale, { style: "percent" })
    return {
      dateTime: (timestamp: Timestamp) => dateTime.format(timestamp),
      /** The day of a timestamp, without the year. */
      day: (timestamp: Timestamp) => day.format(timestamp),
      /** The time of day of a timestamp, to the minute. */
      hour: (timestamp: Timestamp) => hour.format(timestamp),
      number: (value: number) => number.format(value),
      /** A change, with its sign. */
      signed: (value: number) => signed.format(value),
      /** A share from 0 to 1. */
      percent: (value: number) => percent.format(value),
    }
  }, [locale])
}
