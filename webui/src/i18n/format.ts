import { useMemo } from "react"
import { useTranslation } from "react-i18next"

import type { Timestamp } from "@/api/generated/Timestamp"

/** Number and date formatting in the interface language. */
export function useFormatters() {
  const { i18n } = useTranslation()
  const locale = i18n.resolvedLanguage ?? i18n.language

  return useMemo(() => {
    const dateTime = new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" })
    const number = new Intl.NumberFormat(locale)
    return {
      dateTime: (timestamp: Timestamp) => dateTime.format(timestamp),
      number: (value: number) => number.format(value),
    }
  }, [locale])
}
