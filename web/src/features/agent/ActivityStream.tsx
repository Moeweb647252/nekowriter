/**
 * The activity stream (PROJECT §8.6, spec/11 §5.3): a virtualized list of messages, live text and
 * reasoning buffers, tool cards and notices, which sticks to the bottom unless the author scrolled
 * up.
 *
 * Live deltas and previews arrive through the `live` store (spec/11 §4.3); a persisted message
 * replaces the matching buffers (the store drops them when the message event arrives).
 */
import { useEffect, useMemo, useRef } from "react"
import { useTranslation } from "react-i18next"
import { useVirtualizer } from "@tanstack/react-virtual"
import { ChevronRight, Loader2 } from "lucide-react"

import type { InboxItemDto } from "@/api/gen/InboxItemDto"
import type { MessageDto } from "@/api/gen/MessageDto"
import type { MessageWithCallsDto } from "@/api/gen/MessageWithCallsDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { useLiveStore } from "@/stores/live"
import { ToolCallCard } from "@/features/agent/ToolCallCard"
import { cn } from "@/lib/utils"

/** One rendered row of the stream. */
export type Row =
  | { kind: "message"; key: string; message: MessageDto; calls: ToolCallDto[] }
  | { kind: "live"; key: string; text: string; reasoning: string }
  | { kind: "queued"; key: string; item: InboxItemDto }
  | { kind: "divider"; key: string; label: string }

/**
 * How far from the end (CSS pixels) still counts as "at the bottom". A programmatic scroll lands
 * exactly at the end, so only the author can move the view out of this band.
 */
export const STICK_THRESHOLD_PX = 80

/**
 * The stick-to-bottom decision for one scroll position: the view follows new content only while
 * the author is (nearly) at the bottom. Pure so the rule is testable without a layout engine.
 */
export function isNearBottom(metrics: {
  scrollHeight: number
  scrollTop: number
  clientHeight: number
}): boolean {
  return metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight < STICK_THRESHOLD_PX
}

/**
 * What the stick-to-bottom effect has to react to.
 *
 * `rows.length` alone misses the streaming case: the last row keeps growing while its text arrives
 * (`live` buffers, and tool cards whose result streams in), so no new row is added and the view
 * would stop following until the next message. Including the tail's content makes the effect run
 * again on every growth, not only on every append.
 */
export function tailSignature(rows: Row[]): string {
  const last = rows.at(-1)
  if (last === undefined) {
    return ""
  }
  switch (last.kind) {
    case "live":
      return `live:${last.key}:${last.text.length}:${last.reasoning.length}`
    case "message":
      return `message:${last.key}:${last.message.blocks
        .map((block) => block.text.length)
        .join(",")}:${last.calls.map((call) => `${call.id}:${call.status}`).join(",")}`
    case "queued":
      return `queued:${last.key}:${last.item.content.length}`
    case "divider":
      return `divider:${last.key}`
  }
}

/** The rows of the stream: persisted messages, the live buffers of the current step and queued author messages. */
export function buildRows(
  messages: MessageWithCallsDto[],
  live: { seq: number; text: Record<number, string>; reasoning: Record<number, string> } | undefined,
  t: (key: string) => string,
  inbox: InboxItemDto[] = [],
): Row[] {
  const rows: Row[] = []
  for (const item of messages) {
    rows.push({
      kind: "message",
      key: `m-${item.message.id}`,
      message: item.message,
      calls: item.tool_calls,
    })
  }
  if (live !== undefined) {
    const text = Object.values(live.text).join("")
    const reasoning = Object.values(live.reasoning).join("")
    if (text !== "" || reasoning !== "") {
      rows.push({ kind: "live", key: `live-${live.seq}`, text, reasoning })
    }
  }
  // The author's steering message is queued in the inbox; the model has not seen it yet, so it
  // belongs below the persisted history and the current live step.
  for (const item of inbox) {
    rows.push({ kind: "queued", key: `q-${item.id}`, item })
  }
  if (rows.length === 0) {
    rows.push({ kind: "divider", key: "empty", label: t("agent:stream.empty") })
  }
  return rows
}

export function ActivityStream({
  session,
  messages,
  pendingPreview,
  paneName,
  inbox,
}: {
  session: SessionDto
  messages: MessageWithCallsDto[]
  /** Unconsumed author messages from the session snapshot (spec/10 §2.7). */
  inbox: InboxItemDto[]
  /** A rendered line for the tool whose arguments are streaming (spec/03 §12). */
  pendingPreview?: string
  /**
   * Marks this element as a named scroll pane (`data-scroll-pane`). The conception studio passes
   * `"conversation"` so the pane can be addressed in tests and by hand: the studio's document
   * column scrolls on its own, and neither shares this element's scroll position (PROJECT §8.7).
   */
  paneName?: string
}) {
  const { t } = useTranslation()
  const live = useLiveStore((state) => state.streams[session.id])
  const rows = useMemo(() => buildRows(messages, live, t, inbox), [messages, live, t, inbox])
  const tail = tailSignature(rows)
  const parentRef = useRef<HTMLDivElement>(null)
  // The view follows new content until the author scrolls up; it re-pins when they return to the
  // bottom (PROJECT §8.6: "auto-scroll unless the author scrolled up").
  const stick = useRef(true)

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 120,
    overscan: 8,
  })

  useEffect(() => {
    const element = parentRef.current
    if (element === null) {
      return
    }
    const onScroll = () => {
      stick.current = isNearBottom(element)
    }
    element.addEventListener("scroll", onScroll, { passive: true })
    return () => element.removeEventListener("scroll", onScroll)
  }, [])

  // Follow the tail. This runs on every row append **and** on every growth of the last row's
  // content (`tail`), which is what streaming produces: the typing indicator and its growing text
  // change without a row being added. The write is deferred one frame, so the row already has its
  // new height in the DOM; `scrollToEnd` targets the element's own max scroll offset (a stale
  // measurement cannot make it stop short) and its reconcile loop retargets until the layout
  // settles.
  useEffect(() => {
    if (!stick.current || rows.length === 0) {
      return
    }
    const frame = requestAnimationFrame(() => {
      if (stick.current) {
        virtualizer.scrollToEnd()
      }
    })
    return () => cancelAnimationFrame(frame)
  }, [tail, rows.length, virtualizer])

  return (
    <div
      ref={parentRef}
      data-testid="stream"
      data-scroll-pane={paneName}
      className="min-h-0 flex-1 overflow-y-auto px-3 py-2"
    >
      <div
        style={{
          height: `${virtualizer.getTotalSize()}px`,
          width: "100%",
          position: "relative",
        }}
      >
        {virtualizer.getVirtualItems().map((virtualRow) => {
          const row = rows[virtualRow.index]
          return (
            <div
              key={row.key}
              data-index={virtualRow.index}
              ref={virtualizer.measureElement}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${virtualRow.start}px)`,
              }}
              className="pb-2"
            >
              <RowView row={row} sessionId={session.id} pendingPreview={pendingPreview} />
            </div>
          )
        })}
      </div>
    </div>
  )
}

function RowView({
  row,
  sessionId,
  pendingPreview,
}: {
  row: Row
  sessionId: number
  pendingPreview?: string
}) {
  const { t } = useTranslation()

  if (row.kind === "divider") {
    return (
      <div className="py-6 text-center text-xs text-muted-foreground">{row.label}</div>
    )
  }

  if (row.kind === "queued") {
    return (
      <div
        className={cn(
          "space-y-1 border-l-2 border-dashed border-muted-foreground/40 pl-2 opacity-70",
        )}
      >
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <span className="font-medium">{t("agent:stream.author")}</span>
          <span>{t("agent:stream.queued")}</span>
        </div>
        <div className="prose prose-sm max-w-none text-sm whitespace-pre-wrap dark:prose-invert">
          {row.item.content}
        </div>
      </div>
    )
  }

  if (row.kind === "live") {
    return (
      <div className="space-y-1">
        <span className="text-xs text-muted-foreground">
          {t("agent:stream.typing")}
          <Loader2 aria-hidden="true" className="ml-1 inline size-3 animate-spin" />
        </span>
        {row.reasoning === "" ? null : (
          <details className="rounded border bg-muted/30 p-2 text-xs">
            <summary className="cursor-pointer text-muted-foreground">
              {t("agent:stream.thinking")}
            </summary>
            <pre className="mt-1 whitespace-pre-wrap text-muted-foreground">{row.reasoning}</pre>
          </details>
        )}
        {row.text === "" ? null : (
          <div className="prose prose-sm max-w-none text-sm whitespace-pre-wrap dark:prose-invert">
            {row.text}
          </div>
        )}
        {pendingPreview === undefined || pendingPreview === "" ? null : (
          <pre className="rounded border border-dashed bg-muted/30 p-2 text-xs whitespace-pre-wrap">
            {pendingPreview}
          </pre>
        )}
      </div>
    )
  }

  const { message, calls } = row
  const isAuthor = message.role === "user"

  return (
    <div className={cn("space-y-1", isAuthor && "border-l-2 border-primary/40 pl-2")}>
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <span className="font-medium">
          {isAuthor ? t("agent:stream.author") : t("agent:stream.agent")}
        </span>
        <span>{t(`agent:message_kind.${message.kind}`)}</span>
        {message.stop_reason === null ? null : (
          <span className="rounded bg-muted px-1 font-mono">{message.stop_reason}</span>
        )}
      </div>
      {message.blocks.map((block, index) =>
        block.type === "reasoning" ? (
          <details key={`r-${index}`} className="rounded border bg-muted/30 p-2">
            <summary className="flex cursor-pointer items-center gap-1 text-xs text-muted-foreground">
              <ChevronRight aria-hidden="true" className="size-3" />
              {t("agent:stream.thinking")}
            </summary>
            <pre className="mt-1 text-xs whitespace-pre-wrap text-muted-foreground">
              {block.text}
            </pre>
          </details>
        ) : (
          <div
            key={`t-${index}`}
            className="prose prose-sm max-w-none text-sm whitespace-pre-wrap dark:prose-invert"
          >
            {block.text}
          </div>
        ),
      )}
      {calls.length === 0
        ? null
        : calls.map((call) => (
            <ToolCallCard
              key={call.id}
              call={call}
              sessionId={sessionId}
              seq={message.seq}
              defaultOpen={call.status === "held" || call.status === "awaiting_input"}
            />
          ))}
    </div>
  )
}
