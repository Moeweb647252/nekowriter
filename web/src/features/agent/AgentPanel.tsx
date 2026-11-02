/**
 * The agent panel (PROJECT §8.6, spec/11 §5.3).
 *
 * It chooses the session (pinned, or the book's most recently updated), fetches the snapshot and
 * renders the header, controls, writer switches, activity stream, composer and usage footer.
 *
 * Live deltas and previews arrive over the session SSE events (spec/10 §3) and land in the `live`
 * store; a persisted message supersedes its buffers, so the stream never shows text twice.
 */
import { useCallback, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2 } from "lucide-react"

import { useChapter } from "@/api/hooks/chapters"
import { useBookSessions, useSession } from "@/api/hooks/sessions"
import {
  usePatchSession,
  useResumeSession,
  useSendSessionMessage,
  useSessionControl,
} from "@/api/hooks/sessions"
import { useStartDebugSession } from "@/api/hooks/sessions"
import { debugSessionsEnabled } from "@/api/endpoints/sessions"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { Granularity } from "@/api/gen/Granularity"
import type { SessionDto } from "@/api/gen/SessionDto"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { ActivityStream } from "@/features/agent/ActivityStream"
import { Composer } from "@/features/agent/Composer"
import { findHeldCall, heldDecisionSurface } from "@/features/agent/heldCall"
import { SessionControls } from "@/features/agent/SessionControls"
import { SessionErrorNotice } from "@/features/agent/SessionErrorNotice"
import { SessionHeader } from "@/features/agent/SessionHeader"
import { UsageFooter } from "@/features/agent/UsageFooter"
import { AutoPauseSwitch, GranularityControl } from "@/features/agent/WriterControls"
import { useSessionToasts } from "@/features/agent/useSessionToasts"
import { useHotkey } from "@/lib/hotkeys"
import { useErrorToast } from "@/components/common/ErrorToast"
import { useUiStore } from "@/stores/ui"

/** Statuses whose session is "open" (spec/03 §3). */
const OPEN_STATUSES = new Set([
  "idle",
  "running",
  "held",
  "awaiting_input",
  "paused",
  "stopped",
  "failed",
  "interrupted",
])

/** The session the panel shows: the pinned one, else the newest open one, else the newest. */
export function pickSession(
  sessions: SessionDto[],
  pinned: SessionDto | undefined,
  pinnedId: number | null,
): SessionDto | undefined {
  if (pinnedId !== null && pinned !== undefined) {
    return pinned
  }
  const open = sessions.find((session) => OPEN_STATUSES.has(session.status))
  return open ?? sessions[0]
}

/**
 * The session the panel opens for a chapter (spec/11 §5.3: "the chapter's open session").
 *
 * `ChapterDto.open_sessions` (spec/10 §2.8) is authoritative when a chapter is open, because it
 * holds exactly that chapter's open writer, reviewer and archivist sessions. The book-wide list
 * is the fallback for views without a chapter (the book overview, the debug entry point).
 */
export function chapterOpenSessions(chapter: ChapterDto | undefined): SessionDto[] {
  return chapter?.open_sessions ?? []
}

/** The pinned session, preferring the chapter's own open sessions over the book-wide list. */
export function pickPinnedSession(
  candidates: SessionDto[],
  sessions: SessionDto[],
  pinnedId: number | null,
): SessionDto | undefined {
  return (
    candidates.find((session) => session.id === pinnedId) ??
    sessions.find((session) => session.id === pinnedId)
  )
}

export function AgentPanel({ bookId, chapterId }: { bookId: number; chapterId?: number }) {
  const { t } = useTranslation()
  const reportError = useErrorToast()
  const sessionsQuery = useBookSessions(bookId)
  const chapterQuery = useChapter(chapterId ?? 0)
  // The panel keeps the chosen session per book in the persisted UI store (spec/11 §4.3, §5.3).
  const pinnedId = useUiStore((state) => state.pinnedSession[bookId] ?? null)
  const setPinnedId = useCallback(
    (sessionId: number | null) => useUiStore.getState().setPinnedSession(bookId, sessionId),
    [bookId],
  )

  const sessions = useMemo(() => sessionsQuery.data ?? [], [sessionsQuery.data])
  // A chapter's own open sessions win: they are exactly what spec/11 §5.3 means by "the
  // chapter's open session", and they exclude other chapters' sessions.
  const chapterSessions = useMemo(
    () => (chapterId === undefined ? [] : chapterOpenSessions(chapterQuery.data)),
    [chapterId, chapterQuery.data],
  )
  const candidates = chapterSessions.length > 0 ? chapterSessions : sessions
  const pinned = useMemo(
    () => pickPinnedSession(candidates, sessions, pinnedId),
    [candidates, sessions, pinnedId],
  )
  const pinnedIsCandidate =
    pinned !== undefined && candidates.some((session) => session.id === pinned.id)
  const chosen = pickSession(candidates, pinnedIsCandidate ? pinned : undefined, pinnedIsCandidate ? pinnedId : null)
  const chosenId = chosen?.id ?? null
  const snapshot = useSession(chosenId)
  const session = snapshot.data?.session ?? chosen
  const messages = snapshot.data?.messages ?? []
  const inbox = snapshot.data?.inbox ?? []

  useSessionToasts(session)

  const control = useSessionControl(bookId)
  const resume = useResumeSession(bookId)
  const send = useSendSessionMessage(bookId)
  const patch = usePatchSession(bookId)
  const debug = useStartDebugSession(bookId)

  const busy =
    control.isPending || resume.isPending || send.isPending || patch.isPending || debug.isPending

  // The "Pausing…"/"Stopping…" hints are derived: they hold only while the session's status has
  // not reacted yet, so no effect has to clear them (React Compiler forbids setState in effects).
  const [requested, setRequested] = useState<{ operation: string; status: string } | null>(null)
  const pausing =
    requested?.operation === "pause" &&
    requested.status === session?.status &&
    session?.status === "running"
  const stopping =
    requested?.operation === "stop" &&
    requested.status === session?.status &&
    session?.status === "running"

  // Alt+P pauses the running session and continues a waiting one (PROJECT §8.13). The hook
  // ignores IME composition, so a Chinese input method never triggers it.
  useHotkey({ key: "p", alt: true }, () => {
    if (session === undefined) {
      return
    }
    const operation: "pause" | "resume" | "continue" | null =
      session.status === "running"
        ? "pause"
        : session.status === "held" || session.status === "paused"
          ? "resume"
          : session.status === "stopped" ||
              session.status === "failed" ||
              session.status === "interrupted"
            ? "continue"
            : null
    if (operation === null) {
      return
    }
    setRequested({ operation, status: session.status })
    control.mutate(
      { sessionId: session.id, operation },
      {
        onSuccess: (updated) => {
          // A control response may arrive after the author picked another session; only keep the
          // pin when this is still the selected session.
          const currentPinned = useUiStore.getState().pinnedSession[bookId] ?? null
          if (currentPinned === null || currentPinned === session.id) {
            setPinnedId(updated.id)
          }
        },
        onError: (error) => {
          setRequested(null)
          reportError(error)
        },
      },
    )
  })


  if (sessionsQuery.isPending) {
    return <Skeleton className="m-3 h-40" />
  }

  if (session === undefined) {
    return (
      <div className="flex h-full flex-col">
        <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
          <p className="text-sm text-muted-foreground">{t("agent:empty")}</p>
          {debugSessionsEnabled ? (
            <Button
              size="sm"
              variant="outline"
              disabled={debug.isPending}
              onClick={() => {
                debug.mutate(undefined, {
                  onSuccess: (result) => setPinnedId(result.session_id),
                  onError: reportError,
                })
              }}
            >
              {t("agent:debug.start")}
            </Button>
          ) : null}
        </div>
      </div>
    )
  }

  const runControl = (operation: "pause" | "resume" | "stop" | "continue" | "abandon") => {
    setRequested({ operation, status: session.status })
    control.mutate(
      { sessionId: session.id, operation },
      {
        onSuccess: (updated) => {
          // A control response may arrive after the author picked another session; only keep the
          // pin when this is still the selected session.
          const currentPinned = useUiStore.getState().pinnedSession[bookId] ?? null
          if (currentPinned === null || currentPinned === session.id) {
            setPinnedId(updated.id)
          }
        },
        onError: (error) => {
          setRequested(null)
          reportError(error)
        },
      },
    )
  }

  const submitFeedback = (feedback: string) => {
    resume.mutate({ sessionId: session.id, feedback }, { onError: reportError })
  }

  // A held session is waiting on a specific tool call (spec/03 §5.2). The snapshot carries it, so
  // the panel can say what is held and offer the two decisions in the tool's own words; a session
  // whose call is outside the snapshot window falls back to the generic controls (HeldCallReview
  // documents that degradation).
  const heldCall = findHeldCall(session, snapshot.data?.tool_calls, snapshot.data?.messages)
  // A held chapter write belongs to the draft pane's dock (D-141): the panel names the hold and
  // links to the draft, and hands neither decision to `SessionControls` or to the composer, so the
  // same call never has two decision surfaces.
  const dockHeld = heldDecisionSurface(heldCall) === "dock"
  // Approve = the documented approve path: `resume` with no feedback, which finalizes the held call
  // with its deferred effects (spec/03 §5.4). Request changes = the same call with the feedback,
  // which revokes those effects and appends the revoke notice.
  const approveHeld = () => {
    resume.mutate({ sessionId: session.id }, { onError: reportError })
  }
  // The draft pane that owns this hold, for the panel's link. The chapter is the session's own, so
  // a pinned session that is not the open chapter's still links to the right page.
  const sessionChapterId = session.chapter_id ?? undefined
  const draftHref =
    sessionChapterId === undefined ? undefined : `/books/${session.book_id}/chapters/${sessionChapterId}`

  const isWriter = session.kind === "chapter_write" || session.kind === "chapter_revise"
  const canConfigure = !["completed", "superseded", "abandoned"].includes(session.status)

  return (
    <div className="flex h-full flex-col border-l bg-sidebar/20">
      <SessionHeader
        session={session}
        sessions={sessions}
        onSelect={setPinnedId}
        pausing={pausing || session.pause_requested}
        stopping={stopping}
        systemPrompt={snapshot.data?.system_prompt ?? ""}
        toolNames={snapshot.data?.tool_names ?? []}
      />
      <SessionControls
        session={session}
        busy={busy}
        onPause={() => runControl("pause")}
        onStop={() => runControl("stop")}
        onContinue={() => runControl(session.status === "held" || session.status === "paused" ? "resume" : "continue")}
        onFeedback={submitFeedback}
        onAbandon={() => runControl("abandon")}
        // The call is always passed: `SessionControls` owns the surface rule (D-141) and renders
        // the pointer to the draft instead of a second wording of the same two mutations.
        heldCall={heldCall}
        onApproveHeld={approveHeld}
        onRequestChanges={submitFeedback}
        chapterHref={draftHref}
      />
      <SessionErrorNotice error={session.error ?? null} />
      {isWriter ? (
        <>
          <GranularityControl
            session={session}
            disabled={busy || !canConfigure}
            onChange={(granularity: Granularity) =>
              patch.mutate(
                { sessionId: session.id, patch: { granularity } },
                { onError: reportError },
              )
            }
          />
          <AutoPauseSwitch
            session={session}
            disabled={busy || !canConfigure}
            onChange={(autoPause) =>
              patch.mutate(
                { sessionId: session.id, patch: { auto_pause: autoPause } },
                { onError: reportError },
              )
            }
          />
        </>
      ) : null}
      {snapshot.isPending ? (
        <div className="flex flex-1 items-center justify-center">
          <Loader2 aria-hidden="true" className="size-4 animate-spin text-muted-foreground" />
        </div>
      ) : (
        <ActivityStream
          session={session}
          messages={messages}
          inbox={inbox}
          nextCursor={snapshot.data?.next_cursor}
        />
      )}
      <Composer
        session={session}
        busy={busy}
        onSend={(text) =>
          send.mutate({ sessionId: session.id, text }, { onError: reportError })
        }
        onFeedback={submitFeedback}
        // The composer steps aside for a hold decided above (or in the dock's draft pane): a
        // `held` session must never have two differently-worded reject paths. A dock-owned hold is
        // `held`, so the composer waits here as well — and its reason names the draft, not "the
        // review above".
        reviewingHeld={heldCall !== undefined && session.status === "held"}
        reviewInDock={dockHeld}
      />
      <UsageFooter session={session} />
      <div className="border-t px-3 py-1">
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="text-xs text-muted-foreground">{t("agent:footer.hint")}</span>
          </TooltipTrigger>
          <TooltipContent>{t("agent:footer.hint_detail")}</TooltipContent>
        </Tooltip>
      </div>
    </div>
  )
}
