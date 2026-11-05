/**
 * The request-changes box, shared by every surface that rejects a held call (spec/03 §5.4, D-137,
 * D-141).
 *
 * Three places ask for the same thing — the agent panel's `HeldCallReview`, the draft pane's
 * `WriterDock`, and the inline `HoldBar` — and all three must obey one contract: the feedback
 * is trimmed, the submit is disabled while nothing is left, and ⌘⏎/Ctrl+⏎ submits without ever
 * firing during an IME composition. The blank case is not cosmetic: the server filters blank
 * feedback to `None` and then **approves** the held call, so a whitespace-only "request changes"
 * would silently do the opposite of what the author asked.
 *
 * The caller owns where the text lives and what happens on submit, because the surface decides
 * whether the box stays open (the bar collapses, the review block closes) — the contract of the box
 * itself is here, once.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { isComposing } from "@/lib/hotkeys"
import { cn } from "@/lib/utils"

export type HeldFeedbackProps = {
  /** The accessible name of the textarea (`agent:held.feedback_label` by default). */
  label?: string
  /** The hint under the box; omitted when the surface has no room for one. */
  hint?: string
  disabled?: boolean
  /** Called with the **trimmed** feedback; never called with an empty string. */
  onSubmit: (feedback: string) => void
  className?: string
}

/** The textarea, its hint and the submit button; never submits a blank request. */
export function HeldFeedback({
  label,
  hint,
  disabled = false,
  onSubmit,
  className,
}: HeldFeedbackProps) {
  const { t } = useTranslation()
  const [feedback, setFeedback] = useState("")
  const trimmed = feedback.trim()
  const submit = () => {
    if (trimmed === "") {
      return
    }
    onSubmit(trimmed)
  }
  return (
    <div className={cn("space-y-2", className)}>
      <Textarea
        value={feedback}
        autoFocus
        rows={3}
        aria-label={label ?? t("agent:held.feedback_label")}
        placeholder={t("agent:feedback.placeholder")}
        onChange={(event) => setFeedback(event.target.value)}
        onKeyDown={(event) => {
          // Ignore IME composition (AGENTS.md §6): Enter confirms Chinese and Japanese input, so a
          // composing Enter must not send the feedback mid-word.
          if (isComposing(event)) {
            return
          }
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault()
            submit()
          }
        }}
      />
      {hint === undefined ? null : <p className="text-xs text-muted-foreground">{hint}</p>}
      <Button size="sm" disabled={disabled || trimmed === ""} onClick={submit}>
        {t("agent:feedback.submit")}
      </Button>
    </div>
  )
}
