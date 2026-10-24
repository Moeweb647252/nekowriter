/**
 * The session controls (PROJECT §8.6, spec/11 §5.3). The available controls depend on the status
 * of spec/03 §3:
 *
 * - running: Pause ("Pausing…" until the status changes), Stop ("Stopping…").
 * - held / paused: Continue, Feedback…, Abandon (nothing is in flight).
 * - awaiting input: Abandon (the form in the stream is the primary action).
 * - idle: the composer replies; Abandon in the menu.
 * - stopped / failed / interrupted: Continue, Abandon.
 * - completed (reviewer only): Discuss lives in the report tab.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Ban, MessageSquarePlus, Pause, Play, Square } from "lucide-react"

import type { SessionDto } from "@/api/gen/SessionDto"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { FeedbackDialog } from "@/features/agent/FeedbackDialog"

type ControlsProps = {
  session: SessionDto
  busy: boolean
  onPause: () => void
  onStop: () => void
  onContinue: () => void
  onFeedback: (feedback: string) => void
  onAbandon: () => void
}

export function SessionControls({
  session,
  busy,
  onPause,
  onStop,
  onContinue,
  onFeedback,
  onAbandon,
}: ControlsProps) {
  const { t } = useTranslation()
  const [feedbackOpen, setFeedbackOpen] = useState(false)
  const status = session.status

  const canPause = status === "running"
  const canStop = status === "running"
  const canContinue =
    status === "held" || status === "paused" || status === "stopped" || status === "failed" || status === "interrupted"
  const canFeedback = status === "held" || status === "paused"
  const canAbandon =
    status !== "completed" && status !== "superseded" && status !== "abandoned"

  return (
    <div className="flex flex-wrap items-center gap-1 border-b px-3 py-2">
      {canContinue ? (
        <Button
          size="sm"
          variant="secondary"
          disabled={busy}
          onClick={() => {
            if (status === "held" || status === "paused") {
              // Held/paused sessions resume; stopped/failed/interrupted ones continue (spec/03 §3).
              onContinue()
            } else {
              onContinue()
            }
          }}
        >
          <Play aria-hidden="true" />
          {t("agent:controls.continue")}
        </Button>
      ) : null}

      {canFeedback ? (
        <Button size="sm" variant="outline" disabled={busy} onClick={() => setFeedbackOpen(true)}>
          <MessageSquarePlus aria-hidden="true" />
          {t("agent:controls.feedback")}
        </Button>
      ) : null}

      {canPause ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button size="sm" variant="outline" disabled={busy} onClick={onPause}>
              <Pause aria-hidden="true" />
              {session.pause_requested ? t("agent:controls.pausing") : t("agent:controls.pause")}
            </Button>
          </TooltipTrigger>
          <TooltipContent>{t("agent:controls.pause_hint")}</TooltipContent>
        </Tooltip>
      ) : null}

      {canStop ? (
        <Button size="sm" variant="outline" disabled={busy} onClick={onStop}>
          <Square aria-hidden="true" />
          {t("agent:controls.stop")}
        </Button>
      ) : null}

      {canAbandon ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button size="sm" variant="ghost" disabled={busy} aria-label={t("agent:controls.more")}>
              ⋯
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={onAbandon} className="text-destructive">
              <Ban aria-hidden="true" />
              {t("agent:controls.abandon")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}

      <FeedbackDialog
        open={feedbackOpen}
        onOpenChange={setFeedbackOpen}
        onSubmit={(feedback) => {
          setFeedbackOpen(false)
          onFeedback(feedback)
        }}
      />
    </div>
  )
}
