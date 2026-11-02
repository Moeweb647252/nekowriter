/**
 * The activity stream (PROJECT §8.6, spec/11 §5.3): a virtualized list of messages, live text and
 * reasoning buffers, tool cards and notices, which sticks to the bottom unless the author scrolled
 * up.
 *
 * Live deltas and previews arrive through the `live` store (spec/11 §4.3); a persisted message
 * replaces the matching buffers (the store drops them when the message event arrives).
 *
 * Scrollback (D-161) also lives here, so the agent panel, the conception studio and the research
 * task page all get it: the snapshot carries the last 200 messages plus a cursor for the page
 * before them, and the stream pages older messages in from the top without moving the reading
 * position.
 */
import { useCallback, useEffect, useMemo, useRef } from "react"
import { useTranslation } from "react-i18next"
import { useVirtualizer } from "@tanstack/react-virtual"
import { ChevronRight, Loader2 } from "lucide-react"

import type { InboxItemDto } from "@/api/gen/InboxItemDto"
import type { MessageDto } from "@/api/gen/MessageDto"
import type { MessageWithCallsDto } from "@/api/gen/MessageWithCallsDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { useOlderMessages } from "@/api/hooks/sessions"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Button } from "@/components/ui/button"
import { useLiveStore } from "@/stores/live"
import { ToolCallCard } from "@/features/agent/ToolCallCard"
import { cn } from "@/lib/utils"

/** How many messages the session snapshot keeps (spec/10 §2.7: "the last 200"). */
export const SNAPSHOT_MESSAGE_LIMIT = 200

/** Scrolling within one estimated row of the top loads the next older page (spec/11 §5.3). */
export const AUTO_LOAD_TOP_PX = 120

/** The scrollback row's state: more pages, loading, a failed page, or the start of the session. */
export type HistoryState = "more" | "loading" | "error" | "start"

/** One rendered row of the stream. */
export type Row =
  | { kind: "history"; key: string; state: HistoryState }
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
    case "history":
      return `history:${last.state}`
    case "divider":
      return `divider:${last.key}`
  }
}

/**
 * The cursor the first older page starts at (spec/10 §2.7: `GET /sessions/:id/messages?cursor=`).
 *
 * The snapshot's own `next_cursor` is the contract. The server currently returns `None` there
 * even when the 200-message window is truncated, so a full window also yields the cursor from its
 * oldest message's `seq` — the exact value the messages endpoint accepts as "everything older".
 * That fallback is harmless once the server fills the field and is what keeps scrollback reachable
 * today.
 */
export function historyCursor(
  nextCursor: string | null | undefined,
  messages: MessageWithCallsDto[],
): string | undefined {
  if (nextCursor !== null && nextCursor !== undefined) {
    return nextCursor
  }
  if (messages.length >= SNAPSHOT_MESSAGE_LIMIT) {
    const oldest = messages[0]
    if (oldest !== undefined) {
      return String(oldest.message.seq)
    }
  }
  return undefined
}

/**
 * Merge the infinite query's pages before the snapshot's messages, oldest first, dropping repeated
 * message ids. `pages` arrive newest page first (the page just before the snapshot, then older
 * ones), so reversing them gives chronological order; a duplicate can only be an overlap with the
 * live window, and either copy renders the same row.
 */
export function mergeHistoryPages(
  pages: MessageWithCallsDto[][],
  snapshot: MessageWithCallsDto[],
): MessageWithCallsDto[] {
  const seen = new Set<number>()
  const merged: MessageWithCallsDto[] = []
  for (const page of [...pages].reverse()) {
    for (const item of page) {
      if (seen.has(item.message.id)) {
        continue
      }
      seen.add(item.message.id)
      merged.push(item)
    }
  }
  for (const item of snapshot) {
    if (seen.has(item.message.id)) {
      continue
    }
    seen.add(item.message.id)
    merged.push(item)
  }
  return merged
}

/** The rows of the stream: scrollback, persisted messages, the live buffers and queued messages. */
export function buildRows(
  messages: MessageWithCallsDto[],
  live: { seq: number; text: Record<number, string>; reasoning: Record<number, string> } | undefined,
  t: (key: string) => string,
  inbox: InboxItemDto[] = [],
  history: HistoryState | undefined = undefined,
): Row[] {
  const rows: Row[] = []
  if (history !== undefined) {
    rows.push({ kind: "history", key: "history", state: history })
  }
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
  nextCursor,
}: {
  session: SessionDto
  messages: MessageWithCallsDto[]
  /** Unconsumed author messages from the session snapshot (spec/10 §2.7). */
  inbox: InboxItemDto[]
  /** The snapshot's scrollback cursor (spec/10 §2.7, spec/11 §5.3). */
  nextCursor?: string | null
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
  const reportError = useErrorToast()
  const live = useLiveStore((state) => state.streams[session.id])
  const initialCursor = historyCursor(nextCursor, messages)
  const older = useOlderMessages(session.id, initialCursor)
  const olderPages = useMemo(
    () => (older.data?.pages ?? []).map((page) => page.items),
    [older.data],
  )
  const olderPageCount = older.data?.pages.length ?? 0
  const mergedMessages = useMemo(
    () => mergeHistoryPages(olderPages, messages),
    [olderPages, messages],
  )

  // A cursor exists while the snapshot window is truncated, and later while a page returned one.
  const hasMore =
    older.data === undefined ? initialCursor !== undefined : older.hasNextPage
  const historyState: HistoryState | undefined =
    messages.length === 0 && olderPages.length === 0 && !hasMore
      ? undefined
      : older.isFetchingNextPage
        ? "loading"
        : older.isFetchNextPageError
          ? "error"
          : hasMore
            ? "more"
            : "start"
  const rows = useMemo(
    () => buildRows(mergedMessages, live, t, inbox, historyState),
    [mergedMessages, live, t, inbox, historyState],
  )
  const tail = tailSignature(rows)
  const parentRef = useRef<HTMLDivElement>(null)
  // The view follows new content until the author scrolls up; it re-pins when they return to the
  // bottom (PROJECT §8.6: "auto-scroll unless the author scrolled up").
  const stick = useRef(true)
  // Scrolling fires many times per gesture; the ref stops a second page request while the first
  // is still in flight, before React Query's own `isFetchingNextPage` has re-rendered.
  const fetchingOlder = useRef(false)
  // Where the reading position was when the page request started, and how tall the content was.
  // After the page commits, the height it added above the snapshot is added to `scrollTop`, so
  // the first previously visible row stays where it was.
  const anchor = useRef<{ scrollTop: number; totalSize: number } | null>(null)

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 120,
    overscan: 8,
    // Measure by row key, not index: prepending a page shifts every index, and index-keyed
    // measurements would give the new top rows the old top rows' heights.
    getItemKey: (index) => rows[index]?.key ?? index,
  })

  const fetchOlder = older.fetchNextPage
  const loadOlder = useCallback(() => {
    if (fetchingOlder.current) {
      return
    }
    if (!hasMore && !older.isFetchNextPageError) {
      return
    }
    const element = parentRef.current
    if (element !== null) {
      anchor.current = { scrollTop: element.scrollTop, totalSize: virtualizer.getTotalSize() }
    }
    fetchingOlder.current = true
    void fetchOlder().finally(() => {
      fetchingOlder.current = false
    })
  }, [fetchOlder, hasMore, older.isFetchNextPageError, virtualizer])
  // The scroll listener is attached once; this ref keeps it pointed at the current loader.
  const loadOlderRef = useRef(loadOlder)
  useEffect(() => {
    loadOlderRef.current = loadOlder
  }, [loadOlder])

  // A failed page shows its inline retry row and reports the localized error once, through the
  // same toast every other screen uses (spec/11 §4.1).
  const scrollbackError = older.isFetchNextPageError ? older.error : null
  useEffect(() => {
    if (scrollbackError !== null) {
      reportError(scrollbackError)
    }
  }, [scrollbackError, reportError])

  useEffect(() => {
    const element = parentRef.current
    if (element === null) {
      return
    }
    const onScroll = () => {
      stick.current = isNearBottom(element)
      // Reaching the top of the loaded history loads the next page (spec/11 §5.3). The loader
      // itself ignores the call when no cursor remains or a request is already running.
      if (element.scrollTop <= AUTO_LOAD_TOP_PX) {
        loadOlderRef.current()
      }
    }
    element.addEventListener("scroll", onScroll, { passive: true })
    return () => element.removeEventListener("scroll", onScroll)
  }, [])

  // After an older page commits, the content above the snapshot got taller. Adding that height to
  // `scrollTop` keeps the row the author was reading at the same screen position (spec/11 §5.3:
  // "anchored so the first visible row does not move").
  useEffect(() => {
    const pending = anchor.current
    if (pending === null) {
      return
    }
    anchor.current = null
    const element = parentRef.current
    if (element === null) {
      return
    }
    const added = virtualizer.getTotalSize() - pending.totalSize
    if (added > 0) {
      element.scrollTop = pending.scrollTop + added
    }
  }, [olderPageCount, virtualizer])

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
              <RowView
                row={row}
                sessionId={session.id}
                pendingPreview={pendingPreview}
                onLoadOlder={loadOlder}
              />
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
  onLoadOlder,
}: {
  row: Row
  sessionId: number
  pendingPreview?: string
  onLoadOlder: () => void
}) {
  const { t } = useTranslation()

  if (row.kind === "history") {
    if (row.state === "loading") {
      return (
        <div className="flex items-center justify-center gap-2 py-3 text-xs text-muted-foreground">
          <Loader2 aria-hidden="true" className="size-3 animate-spin" />
          {t("agent:stream.loading_older")}
        </div>
      )
    }
    if (row.state === "error") {
      return (
        <div className="flex items-center justify-center gap-2 py-3 text-xs text-muted-foreground">
          {t("agent:stream.load_older_error")}
          <Button variant="ghost" size="sm" className="h-6 text-xs" onClick={onLoadOlder}>
            {t("common:retry")}
          </Button>
        </div>
      )
    }
    if (row.state === "more") {
      return (
        <div className="flex justify-center py-2">
          <Button variant="ghost" size="sm" className="text-xs" onClick={onLoadOlder}>
            {t("agent:stream.load_older")}
          </Button>
        </div>
      )
    }
    return (
      <div className="py-4 text-center text-xs text-muted-foreground">
        {t("agent:stream.start")}
      </div>
    )
  }

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
