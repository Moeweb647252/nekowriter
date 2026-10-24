/**
 * The usage footer (PROJECT §8.6, spec/03 §13): cumulative input (with the cached part) and output
 * tokens for the session, plus the step count.
 */
import { useTranslation } from "react-i18next"

import type { SessionDto } from "@/api/gen/SessionDto"
import { formatNumber } from "@/lib/format"

export function UsageFooter({ session }: { session: SessionDto }) {
  const { t, i18n } = useTranslation()
  const usage = session.usage
  const format = (tokens: number) => formatNumber(tokens, i18n.language)
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t px-3 py-1.5 text-xs text-muted-foreground">
      <span>{t("agent:usage.input", { tokens: format(usage.input_tokens) })}</span>
      <span>{t("agent:usage.cached", { tokens: format(usage.cached_input_tokens) })}</span>
      <span>{t("agent:usage.output", { tokens: format(usage.output_tokens) })}</span>
      <span>{t("agent:usage.requests", { count: usage.requests })}</span>
      {usage.estimated ? <span>{t("agent:usage.estimated")}</span> : null}
    </div>
  )
}
