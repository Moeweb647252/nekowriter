/**
 * The header word count: current words against the book's target range (spec/11 §5.2).
 *
 * `projected` is what the chapter would read once a staged agent write is applied (D-139, D-141).
 * The plain number always stays the **confirmed** count — that is what the chapter holds and what
 * the server counts (spec/11 §8) — and the projection is shown next to it in the waiting token with
 * its own label, so the header and the draft pane never disagree about the same chapter. A single
 * blended number would be a count of text the chapter does not have.
 */
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
  /**
   * Words the chapter would read once a held agent write is applied. Rendered as a labelled
   * projection, never as the count itself (D-141).
   */
  projected?: number
}

/** Shows `2,350 / 3,000–4,000` (spec/PROJECT §8.5). */
export function WordCount({
  words,
  targetMin,
  targetMax,
  language = "en",
  className,
  compact,
  projected,
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
    <span className={cn("flex items-center gap-1 text-xs tabular-nums", className)}>
      <span
        className={inRange ? "text-muted-foreground" : "text-status-waiting"}
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
      {projected === undefined ? null : (
        <span
          className="text-status-waiting"
          title={t("chapter:word_count_projected", {
            words: formatNumber(projected, language),
          })}
        >
          {/* An arrow, not a second number: this count is what the chapter *would* hold. */}
          <span aria-hidden="true">→ </span>
          {formatNumber(projected, language)}
          <span className="sr-only"> {t("chapter:word_count_projected_suffix")}</span>
        </span>
      )}
    </span>
  )
}
