/**
 * Ghost text and the hold bar (PROJECT §8.5, spec/11 §5.2).
 *
 * `GhostText` shows the text a writer tool is still generating: muted, with a blinking caret
 * (spec/03 §12). It covers every writer tool whose preview field is `text` — `insert_text`,
 * `insert_paragraphs` and `rewrite_paragraph` — because all three add prose the author wants to see
 * before it lands. The preview is dropped as soon as the persisted tool call arrives, so nothing is
 * ever shown twice.
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
export type Preview = { name: string; field: string; text: string; pinned?: boolean }

/** The writer tools whose preview field is the prose they are about to write (spec/03 §12). */
const TEXT_PREVIEW_TOOLS = ["insert_text", "insert_paragraphs", "rewrite_paragraph"]

/**
 * The paragraph preview of the chapter's running writer session.
 *
 * `appended` mirrors spec/11 §5.2: an `insert_text` preview goes *into* the open tail paragraph
 * (the agent always writes at the end of the draft, spec/05 §4), while `insert_paragraphs` and
 * `rewrite_paragraph` previews are a block of their own. The preview event carries only the tool
 * name, the field and the decoded text — never the target pid, which arrives with the persisted
 * call — so a paragraph tool's ghost cannot be placed under its target; the tool card names it.
 */
export function GhostText({
  preview,
  appended = false,
}: {
  preview: Preview | undefined
  appended?: boolean
}) {
  if (preview === undefined || preview.text === "") {
    return null
  }
  if (preview.field !== "text" || !TEXT_PREVIEW_TOOLS.includes(preview.name)) {
    return null
  }
  if (appended) {
    // Inside the open tail paragraph, so no `<p>` of its own.
    return (
      <span className="ghost-text" aria-live="polite">
        {preview.text}
        {preview.pinned === true ? null : <span className="ghost-caret" aria-hidden="true" />}
      </span>
    )
  }
  return (
    <p className="ghost-text" aria-live="polite">
      {preview.text}
      {preview.pinned === true ? null : <span className="ghost-caret" aria-hidden="true" />}
    </p>
  )
}

export type HoldBarProps = {
  /** True when the held call is a `submit_outline` (the outline variant, spec/11 §5.2). */
  outline: boolean
  /** The paragraph pids the held call touched, for the label. */
  pids: string[]
  busy: boolean
  /**
   * Continue / approve. The caller saves any unsaved outline or notes *before* it calls the resume
   * endpoint, so the agent reads them (spec/11 §5.2); a save that fails aborts the resume there,
   * because resuming anyway would hand the agent the text the author just replaced.
   */
  onContinue: () => void
  onFeedback: (text: string) => void
}

/** The inline bar under the held paragraphs, with an expandable feedback textarea. */
export function HoldBar({ outline, pids, busy, onContinue, onFeedback }: HoldBarProps) {
  const { t } = useTranslation()
  const [feedbackOpen, setFeedbackOpen] = useState(false)
  const [feedback, setFeedback] = useState("")

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
          <Button size="sm" disabled={busy} onClick={onContinue}>
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
                  onFeedback(text)
                }
              }
            }}
          />
          <Button
            size="sm"
            disabled={busy || feedback.trim() === ""}
            onClick={() => onFeedback(feedback.trim())}
          >
            {t("agent:feedback.submit")}
          </Button>
        </div>
      ) : null}
    </div>
  )
}
