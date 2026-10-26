/**
 * The composer (PROJECT §8.6, spec/11 §5.3). Its behavior is labeled by the session state:
 *
 * - running: "Steer the agent" → `POST /message`.
 * - held / paused: "Feedback & continue" → `POST /resume {feedback}`.
 * - idle (conception): "Reply" → `POST /message`.
 * - waiting for a form: disabled with a reason.
 * - anything else: disabled with a reason.
 *
 * The state label is rendered **once**, in the hint line under the textarea, which is the live
 * region. An earlier version also used it as the textarea's `placeholder`, so a disabled reason
 * such as `agent:composer.disabled_continue` appeared twice in a row. The textarea keeps a stable
 * accessible name and points at the hint with `aria-describedby`, so assistive technology reads the
 * reason once instead of receiving the same sentence as name, placeholder and live-region update.
 *
 * Every string goes through `t()`; nothing is concatenated from translated fragments.
 */
import { useId, useState } from "react"
import { useTranslation } from "react-i18next"
import { SendHorizontal } from "lucide-react"

import type { SessionDto } from "@/api/gen/SessionDto"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"

type ComposerMode = "steer" | "feedback" | "reply" | "disabled"

/** Which composer behavior the session's status allows. */
export function composerMode(session: SessionDto): ComposerMode {
  switch (session.status) {
    case "running":
      return "steer"
    case "held":
    case "paused":
      return "feedback"
    case "idle":
      return session.kind === "conception" ? "reply" : "disabled"
    default:
      return "disabled"
  }
}

/** The i18n key of the disabled reason, when the composer is disabled. */
export function disabledReasonKey(session: SessionDto): string {
  switch (session.status) {
    case "awaiting_input":
      return "agent:composer.disabled_form"
    case "stopped":
    case "failed":
    case "interrupted":
      return "agent:composer.disabled_continue"
    case "completed":
    case "superseded":
    case "abandoned":
      return "agent:composer.disabled_done"
    default:
      return "agent:composer.disabled"
  }
}

export function Composer({
  session,
  busy,
  onSend,
  onFeedback,
}: {
  session: SessionDto
  busy: boolean
  onSend: (text: string) => void
  onFeedback: (feedback: string) => void
}) {
  const { t } = useTranslation()
  const [text, setText] = useState("")
  const hintId = useId()
  const mode = composerMode(session)
  const disabled = mode === "disabled" || busy || text.trim() === ""

  const label =
    mode === "steer"
      ? t("agent:composer.steer")
      : mode === "feedback"
        ? t("agent:composer.feedback")
        : mode === "reply"
          ? t("agent:composer.reply")
          : t(disabledReasonKey(session))

  const submit = () => {
    const value = text.trim()
    if (value === "" || mode === "disabled") {
      return
    }
    if (mode === "feedback") {
      onFeedback(value)
    } else {
      onSend(value)
    }
    setText("")
  }

  return (
    <div className="border-t p-2">
      <Textarea
        value={text}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) => {
          // Ignore IME composition (AGENTS.md §6): a plain Enter while composing inserts a
          // character, it must not send. `nativeEvent.isComposing` is the DOM flag; React's
          // synthetic event does not expose it with the required typing.
          if (event.nativeEvent.isComposing) {
            return
          }
          if (event.key === "Enter" && !event.shiftKey) {
            event.preventDefault()
            submit()
          }
        }}
        // A neutral prompt, and none while the composer is disabled: the reason then lives only in
        // the hint line below, which is the live region (it used to be repeated here as the
        // placeholder as well).
        placeholder={mode === "disabled" ? undefined : t("agent:composer.placeholder")}
        rows={2}
        disabled={mode === "disabled"}
        // A stable name, so the state can change without renaming a focused control (a renamed
        // control is announced by some screen readers on top of the live region). The hint line
        // keeps its id and is referenced as the description.
        aria-label={t("agent:composer.label")}
        aria-describedby={hintId}
        className="resize-none"
      />
      <div className="mt-1 flex items-center justify-between gap-2">
        <span id={hintId} className="text-xs text-muted-foreground" aria-live="polite">
          {label}
        </span>
        <Button size="sm" disabled={disabled} onClick={submit}>
          <SendHorizontal aria-hidden="true" />
          {t("agent:composer.send")}
        </Button>
      </div>
    </div>
  )
}
