/**
 * One tool call card (PROJECT §8.6, spec/11 §5.3): icon, a localized one-line summary built from
 * `ui`, status (spinner, held, error, done), and expandable input and result. Appendices are shown
 * under the card as `NoticeItem`s; long-content tools stream their preview.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import type { LucideIcon } from "lucide-react"
import { ChevronDown, ChevronRight, Loader2 } from "lucide-react"

import type { Appendix } from "@/api/gen/Appendix"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { useLiveStore } from "@/stores/live"
import { NoticeItem } from "@/features/agent/NoticeItem"
import { toolRenderer } from "@/features/agent/toolRenderers"
import { cn } from "@/lib/utils"

/**
 * The icon component of a renderer.
 *
 * It is a wrapper rather than an inline `<Icon />` because the icon comes out of the renderer
 * registry at render time, and React (and the compiler lint) require a stable component type.
 */
function ToolIcon({
  icon: Icon,
  className,
}: {
  icon: LucideIcon
  className?: string
}) {
  return <Icon aria-hidden="true" className={className} />
}

/** The status dot of a tool call. */
function StatusIcon({ call }: { call: ToolCallDto }) {
  if (call.status === "pending" || call.status === "executing") {
    return <Loader2 aria-hidden="true" className="size-3.5 animate-spin text-muted-foreground" />
  }
  if (call.status === "held" || call.status === "awaiting_input") {
    return <span aria-hidden="true" className="size-2 rounded-full bg-amber-500" />
  }
  if (call.is_error || call.status === "error") {
    return <span aria-hidden="true" className="size-2 rounded-full bg-destructive" />
  }
  return <span aria-hidden="true" className="size-2 rounded-full bg-emerald-500" />
}

export function ToolCallCard({
  call,
  sessionId,
  seq,
  defaultOpen = false,
  includeInboxAppendices = true,
}: {
  call: ToolCallDto
  sessionId: number
  /** The assistant message sequence, to find the streaming preview of this call. */
  seq: number
  defaultOpen?: boolean
  /**
   * `false` when the caller lifts `inbox` appendices into their own author rows: the card then
   * omits that kind so the author's message is not duplicated inside the collapsed content.
   */
  includeInboxAppendices?: boolean
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(defaultOpen)
  const renderer = toolRenderer(call.name)
  const renderedSummary = renderer.summary(call.ui, t).trim()
  const failureLine =
    (call.result_text ?? "")
      .split("\n")
      .map((line) => line.trim())
      .find((line) => line !== "") ?? ""
  // A failed call with no summary used to show only the tool name (and "读取了" with a blank URL
  // for web_fetch). Name the failure and its first result line instead; the stored row is
  // unchanged.
  const failed = call.is_error || call.status === "error"
  const summary = failed
    ? failureLine === ""
      ? renderedSummary === ""
        ? t("agent:status.failed")
        : renderedSummary
      : t("agent:tool.failed_with", { message: failureLine })
    : renderedSummary
  const appendices = includeInboxAppendices
    ? call.appendices
    : call.appendices.filter((appendix: Appendix) => appendix.kind !== "inbox")
  // A preview is discarded as soon as the persisted call arrives (spec/03 §12).
  const preview = useLiveStore(
    (state) => state.previews[`${sessionId}:${seq}:${call.call_index}`],
  )
  const showPreview = preview !== undefined && preview.text !== "" && call.sealed === false

  return (
    <Collapsible open={open} onOpenChange={setOpen} className="rounded-md border bg-card">
      <CollapsibleTrigger asChild>
        <Button
          variant="ghost"
          className="h-auto w-full justify-start gap-2 px-2 py-1.5 text-left font-normal"
        >
          {open ? (
            <ChevronDown aria-hidden="true" className="size-3.5 shrink-0" />
          ) : (
            <ChevronRight aria-hidden="true" className="size-3.5 shrink-0" />
          )}
          <ToolIcon icon={renderer.icon} className="size-3.5 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1 truncate text-xs">
            <span className="font-mono text-muted-foreground">{call.name}</span>
            {summary === "" ? null : <span className="ml-1.5">{summary}</span>}
          </span>
          <StatusIcon call={call} />
        </Button>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className="space-y-2 border-t px-2 py-2">
          {call.status === "held" ? (
            <Badge variant="outline" className="border-amber-500/40 text-amber-700 dark:text-amber-400">
              {t("agent:tool.held")}
            </Badge>
          ) : null}
          {call.status === "awaiting_input" ? (
            <Badge variant="outline" className="border-sky-500/40 text-sky-700 dark:text-sky-400">
              {t("agent:tool.awaiting_input")}
            </Badge>
          ) : null}

          {showPreview ? (
            <div>
              <div className="text-xs font-medium text-muted-foreground">
                {t("agent:tool.preview")}
              </div>
              <pre className="mt-1 max-h-48 overflow-auto rounded bg-muted p-2 text-xs whitespace-pre-wrap">
                {preview.text}
              </pre>
            </div>
          ) : null}

          <div>
            <div className="text-xs font-medium text-muted-foreground">{t("agent:tool.input")}</div>
            <pre className="mt-1 max-h-40 overflow-auto rounded bg-muted p-2 text-xs whitespace-pre-wrap">
              {formatJson(call.input, call.input_text)}
            </pre>
          </div>

          {call.result_text === null || call.result_text === "" ? null : (
            <div>
              <div className="text-xs font-medium text-muted-foreground">
                {t("agent:tool.result")}
              </div>
              <pre
                className={cn(
                  "mt-1 max-h-64 overflow-auto rounded bg-muted p-2 text-xs whitespace-pre-wrap",
                  call.is_error && "text-destructive",
                )}
              >
                {call.result_text}
              </pre>
            </div>
          )}

          {appendices.length === 0 ? null : (
            <div className="space-y-1">
              <div className="text-xs font-medium text-muted-foreground">
                {t("agent:tool.appendices")}
              </div>
              {appendices.map((appendix: Appendix, index: number) => (
                <NoticeItem key={`${appendix.kind}-${index}`} appendix={appendix} />
              ))}
            </div>
          )}
        </div>
      </CollapsibleContent>
    </Collapsible>
  )
}

/** Formats a tool call's input: the parsed JSON when valid, else the raw text. */
function formatJson(input: unknown, raw: string): string {
  if (input !== null && input !== undefined) {
    return JSON.stringify(input, null, 2)
  }
  return raw
}
