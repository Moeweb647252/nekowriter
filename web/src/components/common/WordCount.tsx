/** The header word count: current words against the book's target range (spec/11 §5.2). */
import { useTranslation } from "react-i18next"

import { formatNumber } from "@/lib/format"
import { cn } from "@/lib/utils"

export type WordCountProps = {
  words: number
  targetMin?: number
  targetMax?: number
  language?: string
  className?: string
  /** Show only the number and the unit, without the target range. */
  compact?: boolean
}

/** Shows `2,350 / 3,000–4,000` (spec/PROJECT §8.5). */
export function WordCount({
  words,
  targetMin,
  targetMax,
  language = "en",
  className,
  compact,
}: WordCountProps) {
  const { t } = useTranslation()
  const current = formatNumber(words, language)
  const inRange =
    targetMin !== undefined && targetMax !== undefined
      ? words >= targetMin && words <= targetMax
      : true
  const title = t("chapter:word_count_title", {
    words: current,
    min: targetMin === undefined ? "" : formatNumber(targetMin, language),
    max: targetMax === undefined ? "" : formatNumber(targetMax, language),
  })
  return (
    <span
      className={cn(
        "text-xs tabular-nums",
        inRange ? "text-muted-foreground" : "text-status-waiting",
        className,
      )}
      title={title}
    >
      {current}
      {compact || targetMin === undefined || targetMax === undefined ? null : (
        <>
          {" / "}
          {formatNumber(targetMin, language)}–{formatNumber(targetMax, language)}
        </>
      )}
      <span className="sr-only"> {t("common:words_unit")}</span>
    </span>
  )
}
