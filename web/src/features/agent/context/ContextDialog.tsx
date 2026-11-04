import { useState } from "react"
import { useTranslation } from "react-i18next"
import { LayoutGrid } from "lucide-react"

import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { useSessionContext } from "@/api/hooks/sessions"
import type { SessionDto } from "@/api/gen/SessionDto"
import { formatNumber } from "@/lib/format"
import { useUiStore } from "@/stores/ui"

import { ContextList } from "./ContextList"
import { ContextTreemap, type ZoomPath } from "./ContextTreemap"
import { RequestStrip } from "./RequestStrip"
import { groupLabel } from "./labels"

export function ContextDialog({
  sessionId,
  open,
  onOpenChange,
}: {
  sessionId: number
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t, i18n } = useTranslation()
  const view = useUiStore((state) => state.contextView)
  const setView = useUiStore((state) => state.setContextView)
  const [selectedSeq, setSelectedSeq] = useState<number | null>(null)
  const [zoom, setZoom] = useState<ZoomPath>({ group: null, part: null })
  const query = useSessionContext(sessionId, selectedSeq, open)
  const data = query.data

  const displayTokens = data === null || data === undefined
    ? 0
    : data.measured ?? Math.round(data.estimate * data.scale)
  const percent = data !== undefined && data !== null && data.context_window > 0
    ? Math.round((displayTokens / data.context_window) * 100)
    : 0
  const source = data === undefined || data === null
    ? ""
    : data.calibrated
      ? selectedSeq === null
        ? t("agent:context.header.scaled")
        : t("agent:context.header.provider")
      : t("agent:context.header.estimate")
  const path = zoom.group === null
    ? [t("agent:context.breadcrumb.root")]
    : [t("agent:context.breadcrumb.root"), groupLabel(t, zoom.group)]

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-5xl" onEscapeKeyDown={(event) => {
        if (zoom.part !== null) {
          event.preventDefault()
          setZoom({ group: zoom.group, part: null })
        } else if (zoom.group !== null) {
          event.preventDefault()
          setZoom({ group: null, part: null })
        }
      }}>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <LayoutGrid aria-hidden="true" className="size-4" />
            {t("agent:context.title")}
          </DialogTitle>
        </DialogHeader>
        {query.isPending && data === undefined ? (
          <div className="py-10 text-center text-sm text-muted-foreground">{t("common:loading", { ns: "common" })}</div>
        ) : query.isError || data === undefined || data === null ? (
          <div className="py-10 text-center text-sm text-muted-foreground">{t("agent:context.empty")}</div>
        ) : (
          <div className="space-y-3">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm">
              <span className="font-medium">
                {t("agent:context.header.tokens", { ns: "agent", estimate: formatNumber(displayTokens, i18n.language), percent, window: formatNumber(data.context_window, i18n.language) })}
              </span>
              <span className="text-xs text-muted-foreground">{source}</span>
              {data.cached !== null ? <span className="text-xs text-muted-foreground">{t("agent:context.header.cached", { ns: "agent", tokens: formatNumber(data.cached, i18n.language) })}</span> : null}
              {data.pending_calls > 0 ? <span className="text-xs text-muted-foreground">{t("agent:context.header.pending", { ns: "agent", count: data.pending_calls })}</span> : null}
            </div>
            <RequestStrip view={data} selectedSeq={selectedSeq} onSelect={(seq) => {
              setSelectedSeq(seq)
              if (seq === null) setZoom({ group: null, part: null })
            }} />
            <div className="flex items-center justify-between gap-2">
              <nav className="flex items-center gap-1 text-sm" aria-label={t("agent:context.title")}>
                {path.map((label, index) => (
                  <button key={`${label}-${index}`} type="button" className="text-muted-foreground hover:text-foreground" onClick={() => setZoom({ group: index === 0 ? null : zoom.group, part: null })}>
                    {index === 0 ? label : `› ${label}`}
                  </button>
                ))}
                {zoom.part !== null ? <span className="text-muted-foreground">› {zoom.part.name ?? ""}</span> : null}
              </nav>
              <div className="flex gap-1 rounded-md border p-0.5 text-xs">
                <button type="button" className={view === "treemap" ? "rounded bg-muted px-2 py-1" : "px-2 py-1"} onClick={() => setView("treemap")}>{t("agent:context.toggle.treemap")}</button>
                <button type="button" className={view === "list" ? "rounded bg-muted px-2 py-1" : "px-2 py-1"} onClick={() => setView("list")}>{t("agent:context.toggle.list")}</button>
              </div>
            </div>
            {view === "treemap" ? <ContextTreemap view={data} zoom={zoom} onZoom={setZoom} onReset={() => setZoom({ group: null, part: null })} /> : <ContextList view={data} />}
            <div className="flex flex-wrap gap-4 text-xs text-muted-foreground">
              <span className="flex items-center gap-1"><span className="size-2 rounded-full bg-[var(--context-setup)]" />{t("agent:context.legend.setup")}</span>
              <span className="flex items-center gap-1"><span className="size-2 rounded-full bg-[var(--context-messages)]" />{t("agent:context.legend.messages")}</span>
              <span className="flex items-center gap-1"><span className="size-2 rounded-full bg-[var(--context-work)]" />{t("agent:context.legend.work")}</span>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}

export function ContextButton({ session, showShare = false }: { session: SessionDto; showShare?: boolean }) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <>
      <button type="button" className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground" onClick={() => setOpen(true)}>
        <LayoutGrid aria-hidden="true" className="size-3.5" />
        {t("agent:context.button")}
        {showShare && session.context_window > 0 ? (
          <span className="tabular-nums">
            {Math.round((session.usage.last_prompt_tokens / session.context_window) * 100)}%
          </span>
        ) : null}
      </button>
      {open ? <ContextDialog sessionId={session.id} open={open} onOpenChange={setOpen} /> : null}
    </>
  )
}
