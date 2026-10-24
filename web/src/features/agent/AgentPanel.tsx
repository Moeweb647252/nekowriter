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
import { ChevronDown, Loader2 } from "lucide-react"

import { useBookSessions, useSession } from "@/api/hooks/sessions"
import {
  usePatchSession,
  useResumeSession,
  useSendSessionMessage,
  useSessionControl,
} from "@/api/hooks/sessions"
import { useStartDebugSession } from "@/api/hooks/sessions"
import { debugSessionsEnabled } from "@/api/endpoints/sessions"
import type { Granularity } from "@/api/gen/Granularity"
import type { SessionDto } from "@/api/gen/SessionDto"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { ActivityStream } from "@/features/agent/ActivityStream"
import { Composer } from "@/features/agent/Composer"
import { SessionControls } from "@/features/agent/SessionControls"
import { SessionHeader } from "@/features/agent/SessionHeader"
import { UsageFooter } from "@/features/agent/UsageFooter"
import { AutoPauseSwitch, GranularityControl } from "@/features/agent/WriterControls"
import { useSessionToasts } from "@/features/agent/useSessionToasts"
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

export function AgentPanel({ bookId }: { bookId: number }) {
  const { t } = useTranslation()
  const reportError = useErrorToast()
  const sessionsQuery = useBookSessions(bookId)
  // The panel keeps the chosen session per book in the persisted UI store (spec/11 §4.3, §5.3).
  const pinnedId = useUiStore((state) => state.pinnedSession[bookId] ?? null)
  const setPinnedId = useCallback(
    (sessionId: number | null) => useUiStore.getState().setPinnedSession(bookId, sessionId),
    [bookId],
  )

  const sessions = useMemo(() => sessionsQuery.data ?? [], [sessionsQuery.data])
  const pinned = useMemo(
    () => sessions.find((session) => session.id === pinnedId),
    [sessions, pinnedId],
  )
  const chosen = pickSession(sessions, pinned, pinnedId)
  const chosenId = chosen?.id ?? null
  const snapshot = useSession(chosenId)
  const session = snapshot.data?.session ?? chosen
  const messages = snapshot.data?.messages ?? []

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
        onSuccess: (updated) => setPinnedId(updated.id),
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
      />
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
        <ActivityStream session={session} messages={messages} />
      )}
      {snapshot.data !== undefined && snapshot.data.next_cursor !== null ? (
        <div className="border-t px-3 py-1">
          <Button variant="ghost" size="sm" className="w-full text-xs">
            <ChevronDown aria-hidden="true" />
            {t("agent:stream.load_older")}
          </Button>
        </div>
      ) : null}
      <Composer
        session={session}
        busy={busy}
        onSend={(text) =>
          send.mutate({ sessionId: session.id, text }, { onError: reportError })
        }
        onFeedback={submitFeedback}
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
