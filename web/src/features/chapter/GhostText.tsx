/**
 * Ghost text and the hold bar (PROJECT §8.5, spec/11 §5.2).
 *
 * `GhostText` shows the text of an `insert_text` the writer is still generating: muted, with a
 * blinking caret, appended to the open tail paragraph or as a new block (spec/03 §12). The preview
 * is dropped as soon as the persisted tool call arrives, so nothing is ever shown twice.
 *
 * `HoldBar` appears under the paragraphs a held call touched (from `ToolCallDto.ui`) with Continue
 * and Feedback; for a held `submit_outline` it renders the outline variant instead.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Check, Pencil, Pause } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { isComposing } from "@/lib/hotkeys"

/** The live preview of one in-flight tool call (spec/11 §4.3 `live.previews`). */
export type Preview = { name: string; field: string; text: string }

/**
 * The paragraph preview of the chapter's running writer session.
 *
 * An `insert_text` preview is appended to the last paragraph: the agent always writes at the end of
 * the draft (spec/05 §4). The other text tools name their target in the tool card instead.
 */
export function GhostText({ preview }: { preview: Preview | undefined }) {
  if (preview === undefined || preview.text === "") {
    return null
  }
  if (preview.name !== "insert_text" || preview.field !== "text") {
    return null
  }
  return (
    <p className="ghost-text" aria-live="polite">
      {preview.text}
      <span className="ghost-caret" aria-hidden="true" />
    </p>
  )
}

export type HoldBarProps = {
  /** True when the held call is a `submit_outline` (the outline variant, spec/11 §5.2). */
  outline: boolean
  /** The paragraph pids the held call touched, for the label. */
  pids: string[]
  busy: boolean
  onContinue: () => void
  onFeedback: (text: string) => void
  /** Saves any unsaved outline/notes edits before the resume call (spec/11 §5.2). */
  onBeforeContinue?: () => Promise<void>
}

/** The inline bar under the held paragraphs, with an expandable feedback textarea. */
export function HoldBar({
  outline,
  pids,
  busy,
  onContinue,
  onFeedback,
  onBeforeContinue,
}: HoldBarProps) {
  const { t } = useTranslation()
  const [feedbackOpen, setFeedbackOpen] = useState(false)
  const [feedback, setFeedback] = useState("")

  const run = (action: () => void) => {
    if (onBeforeContinue === undefined) {
      action()
      return
    }
    // A failed save must not swallow the resume the author asked for.
    void onBeforeContinue().then(action, action)
  }

  const label = outline
    ? t("chapter:hold.outline_label")
    : pids.length > 0
      ? t("chapter:hold.text_label_pids", { pids: pids.join(", ") })
      : t("chapter:hold.text_label")

  return (
    <div className="hold-bar" role="group" aria-label={label}>
      <div className="flex flex-wrap items-center gap-2">
        <span className="flex items-center gap-1.5 text-sm text-muted-foreground">
          <Pause className="size-3.5" aria-hidden="true" />
          {label}
        </span>
        <div className="ml-auto flex items-center gap-2">
          <Button size="sm" disabled={busy} onClick={() => run(onContinue)}>
            <Check className="size-4" aria-hidden="true" />
            {outline ? t("chapter:hold.approve_outline") : t("agent:controls.continue")}
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => {
              // Collapsing drops the unsent text, so a later expand starts clean.
              setFeedbackOpen((open) => {
                if (open) {
                  setFeedback("")
                }
                return !open
              })
            }}
          >
            <Pencil className="size-4" aria-hidden="true" />
            {outline ? t("chapter:hold.request_changes") : t("agent:controls.feedback")}
          </Button>
        </div>
      </div>
      {feedbackOpen ? (
        <div className="mt-2 space-y-2">
          <Textarea
            value={feedback}
            autoFocus
            rows={3}
            placeholder={t("agent:feedback.placeholder")}
            onChange={(event) => setFeedback(event.target.value)}
            onKeyDown={(event) => {
              if (isComposing(event)) {
                return
              }
              if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
                event.preventDefault()
                const text = feedback.trim()
                if (text !== "") {
                  run(() => onFeedback(text))
                }
              }
            }}
          />
          <Button
            size="sm"
            disabled={busy || feedback.trim() === ""}
            onClick={() => run(() => onFeedback(feedback.trim()))}
          >
            {t("agent:feedback.submit")}
          </Button>
        </div>
      ) : null}
    </div>
  )
}
