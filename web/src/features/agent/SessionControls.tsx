/**
 * The session controls (PROJECT §8.6, spec/11 §5.3). The available controls depend on the status
 * of spec/03 §3:
 *
 * - running: Pause ("Pausing…" until the status changes), Stop ("Stopping…").
 * - held: when the held call is known, the review block below the bar (see `HeldCallReview`) —
 *   Approve / Request changes in the tool's own words, with Abandon still in the menu. Without a
 *   resolvable call it degrades to Continue, Feedback…, Abandon.
 * - paused: Continue, Feedback…, Abandon (nothing is in flight).
 * - awaiting input: Abandon (the form in the stream is the primary action).
 * - idle: the composer replies; Abandon in the menu.
 * - stopped / failed / interrupted: Continue, Abandon.
 * - completed (reviewer only): Discuss lives in the report tab.
 *
 * A `held` session pauses **on a tool call** whose deferred effects wait for the author
 * (spec/03 §5.2, §5.4). A bare Continue therefore asks the author to approve something the bar
 * never names; the review block takes over those two decisions whenever the caller can say which
 * call is held.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Ban, MessageSquarePlus, Pause, Play, Square } from "lucide-react"

import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { ConfirmDialog } from "@/components/common/ConfirmDialog"
import { FeedbackDialog } from "@/features/agent/FeedbackDialog"
import { HeldCallReview } from "@/features/agent/HeldCallReview"

type ControlsProps = {
  session: SessionDto
  busy: boolean
  onPause: () => void
  onStop: () => void
  onContinue: () => void
  onFeedback: (feedback: string) => void
  onAbandon: () => void
  /**
   * The call a `held` session is waiting on, resolved from the snapshot by the caller. When it is
   * present the review block owns the approve/request-changes decisions (spec/03 §5.4);
   * `undefined` keeps the status-driven controls, which is what a surface that cannot resolve the
   * call gets.
   */
  heldCall?: ToolCallDto
  /** Approve the held call: `resume` with no feedback. */
  onApproveHeld?: () => void
  /** Request changes on the held call: `resume` with exactly this feedback. */
  onRequestChanges?: (feedback: string) => void
}

export function SessionControls({
  session,
  busy,
  onPause,
  onStop,
  onContinue,
  onFeedback,
  onAbandon,
  heldCall,
  onApproveHeld,
  onRequestChanges,
}: ControlsProps) {
  const { t } = useTranslation()
  const [feedbackOpen, setFeedbackOpen] = useState(false)
  // Abandoning ends the session for good, so it is confirmed like every destructive action
  // (PROJECT §8.13).
  const [confirmAbandon, setConfirmAbandon] = useState(false)
  const status = session.status

  const canPause = status === "running"
  const canStop = status === "running"
  // The review block owns a held session's two decisions when the caller can name the call, so the
  // same choice is never offered twice under two labels. Anything else keeps the generic pair.
  const reviewHeld =
    status === "held" &&
    heldCall !== undefined &&
    onApproveHeld !== undefined &&
    onRequestChanges !== undefined
  const genericHeld = status === "held" && !reviewHeld
  const canContinue =
    genericHeld ||
    status === "paused" ||
    status === "stopped" ||
    status === "failed" ||
    status === "interrupted"
  const canFeedback = genericHeld || status === "paused"
  const canAbandon =
    status !== "completed" && status !== "superseded" && status !== "abandoned"

  return (
    <div className="flex shrink-0 flex-col">
      <div className="flex flex-wrap items-center gap-1 border-b px-3 py-2">
        {canContinue ? (
          <Button size="sm" variant="secondary" disabled={busy} onClick={onContinue}>
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
              <DropdownMenuItem
                onSelect={() => setConfirmAbandon(true)}
                className="text-destructive"
              >
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
        <ConfirmDialog
          open={confirmAbandon}
          onOpenChange={setConfirmAbandon}
          title={t("agent:controls.abandon_title")}
          description={t("agent:controls.abandon_body")}
          confirmLabel={t("agent:controls.abandon_confirm")}
          pending={busy}
          onConfirm={() => {
            setConfirmAbandon(false)
            onAbandon()
          }}
        />
      </div>
      {reviewHeld ? (
        <HeldCallReview
          call={heldCall}
          busy={busy}
          onApprove={onApproveHeld}
          onRequestChanges={onRequestChanges}
        />
      ) : null}
    </div>
  )
}
