import { useTranslation } from "react-i18next"

import type { ContextBreakdownDto } from "@/api/gen/ContextBreakdownDto"
import { formatNumber } from "@/lib/format"

export function RequestStrip({
  view,
  selectedSeq,
  onSelect,
}: {
  view: ContextBreakdownDto
  selectedSeq: number | null
  onSelect: (seq: number | null) => void
}) {
  const { t, i18n } = useTranslation()
  const language = i18n.language
  const entries = [
    ...view.requests.map((request) => ({
      seq: request.seq,
      tokens: request.estimated ? request.estimate : request.prompt_tokens,
      label: `Step ${request.seq}`,
      cached: request.cached_tokens,
      createdAt: request.created_at,
    })),
    {
      seq: null,
      tokens: Math.round(view.estimate * view.scale),
      label: t("agent:context.strip.current"),
      cached: null,
      createdAt: null,
    },
  ]
  const max = Math.max(view.context_window, ...entries.map((entry) => entry.tokens), 1)
  const currentIndex = entries.length - 1
  const selectedIndex = selectedSeq === null ? currentIndex : entries.findIndex((entry) => entry.seq === selectedSeq)
  const selected = entries[selectedIndex] ?? entries[currentIndex]
  const move = (delta: number) => {
    const next = Math.max(0, Math.min(entries.length - 1, selectedIndex + delta))
    onSelect(entries[next]?.seq ?? null)
  }

  return (
    <div className="space-y-1">
      <div className="relative">
      <div
        role="slider"
        tabIndex={0}
        aria-label={t("agent:context.title")}
        aria-valuemin={1}
        aria-valuemax={entries.length}
        aria-valuenow={selectedIndex + 1}
        aria-valuetext={`Step ${selected?.seq ?? "current"} of ${entries.length}, ${formatNumber(selected?.tokens ?? 0, language)} tokens`}
        className="relative flex h-20 items-end gap-px overflow-x-auto pb-1 outline-none focus-visible:ring-2 focus-visible:ring-ring"
        onKeyDown={(event) => {
          if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
            event.preventDefault()
            move(-1)
          } else if (event.key === "ArrowRight" || event.key === "ArrowDown") {
            event.preventDefault()
            move(1)
          } else if (event.key === "Home") {
            event.preventDefault()
            onSelect(entries[0]?.seq ?? null)
          } else if (event.key === "End") {
            event.preventDefault()
            onSelect(null)
          }
        }}
      >
        {entries.map((entry, index) => {
          const isCurrent = entry.seq === null
          const active = index === selectedIndex
          return (
            <button
              key={entry.seq ?? "current"}
              type="button"
              tabIndex={-1}
              aria-label={entry.label}
              title={`${entry.label} · ${formatNumber(entry.tokens, language)} tokens${entry.cached === null ? "" : ` · cached ${formatNumber(entry.cached, language)}`}`}
              onClick={() => onSelect(entry.seq)}
              className="group flex shrink-0 flex-col items-center justify-end"
              style={{ width: Math.max(3, Math.min(24, 800 / Math.max(entries.length, 1))) }}
            >
              <span
                className={`w-full rounded-t ${active ? "bg-foreground" : "bg-muted-foreground/40"} ${isCurrent ? "border border-dashed border-foreground bg-transparent" : ""}`}
                style={{ height: `${Math.max(3, Math.round((entry.tokens / max) * 72))}px` }}
              />
            </button>
          )
        })}
        <span
          aria-hidden="true"
          className="pointer-events-none absolute inset-y-0 border-l border-foreground/40"
          style={{ left: `${(view.context_window / max) * 100}%` }}
        />
        {view.rollover_at !== null ? (
          <span
            aria-hidden="true"
            className="pointer-events-none absolute inset-y-0 border-l border-dashed border-foreground"
            style={{ left: `${(view.rollover_at / max) * 100}%` }}
          />
        ) : null}
      </div>
      <div className="flex justify-between text-[10px] text-muted-foreground">
        <span>{t("agent:context.strip.current")}</span>
        {view.rollover_at !== null ? <span>{t("agent:context.strip.renews")}</span> : null}
      </div>
      </div>
    </div>
  )
}
