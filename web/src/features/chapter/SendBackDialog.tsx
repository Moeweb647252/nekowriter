/**
 * The send-back dialog (PROJECT §8.5): an editable feedback textarea, checkboxes for the issues
 * to forward, and the mode radio (revise / redraft). The ticked issues travel as `issue_indexes`
 * and are copied into `chapter_feedback.issues` by the API (spec/08 §1.1, §3).
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { useSendBack } from "@/api/hooks/reviews"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { Textarea } from "@/components/ui/textarea"
import type { FeedbackMode } from "@/api/gen/FeedbackMode"
import type { ReviewReportDto } from "@/api/gen/ReviewReportDto"
import { defaultTicked, tickedIssues } from "@/features/chapter/reviewUtils"

export type SendBackDialogProps = {
  chapterId: number
  bookId: number
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The report the dialog was opened from: prefills the feedback and lists its issues. */
  report?: ReviewReportDto
  /** Overrides the prefilled feedback (the author's edited text in the Review tab). */
  initialFeedback?: string
  onSentBack?: (sessionId: number) => void
}

export function SendBackDialog({
  chapterId,
  bookId,
  open,
  onOpenChange,
  report,
  initialFeedback,
  onSentBack,
}: SendBackDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        {open ? (
          // Remounted on every opening, so the form starts from the report it was opened from
          // without a state-syncing effect.
          <SendBackForm
            key={`${report?.id ?? "none"}:${initialFeedback ?? ""}`}
            chapterId={chapterId}
            bookId={bookId}
            report={report}
            initialFeedback={initialFeedback}
            onOpenChange={onOpenChange}
            onSentBack={onSentBack}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  )
}

/** The dialog body; state is initialized from the report it opened with. */
function SendBackForm({
  chapterId,
  bookId,
  report,
  initialFeedback,
  onOpenChange,
  onSentBack,
}: Omit<SendBackDialogProps, "open">) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const sendBack = useSendBack(chapterId, bookId)
  const issues = report?.issues ?? []
  const [feedback, setFeedback] = useState(initialFeedback ?? report?.feedback ?? "")
  const [mode, setMode] = useState<FeedbackMode>("revise")
  const [ticked, setTicked] = useState<Set<number>>(() => defaultTicked(issues.length))

  const toggle = (index: number, checked: boolean) => {
    setTicked((current) => {
      const next = new Set(current)
      if (checked) {
        next.add(index)
      } else {
        next.delete(index)
      }
      return next
    })
  }

  const selected = tickedIssues(issues, ticked)

  const submit = () => {
    sendBack.mutate(
      {
        feedback,
        mode,
        review_report_id: report?.id,
        issue_indexes: issues
          .map((_issue, index) => index)
          .filter((index) => ticked.has(index)),
      },
      {
        onSuccess: (response) => {
          onOpenChange(false)
          onSentBack?.(response.session_id)
        },
        onError: showError,
      },
    )
  }

  return (
    <>
      <DialogHeader>
          <DialogTitle>{t("chapter:send_back_dialog.title")}</DialogTitle>
          <DialogDescription>
            {t("chapter:send_back_dialog.description")}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label htmlFor="send-back-feedback" className="text-xs">
              {t("chapter:send_back_dialog.feedback_label")}
            </Label>
            <Textarea
              id="send-back-feedback"
              rows={6}
              value={feedback}
              placeholder={t("chapter:send_back_dialog.feedback_placeholder")}
              onChange={(event) => setFeedback(event.target.value)}
            />
          </div>

          {issues.length === 0 ? null : (
            <div className="space-y-1.5">
              <div className="text-xs font-medium">
                {t("chapter:send_back_dialog.issues_label")}
              </div>
              <ul className="max-h-48 space-y-1 overflow-y-auto rounded-md border p-2">
                {issues.map((issue, index) => (
                  <li key={index} className="flex items-start gap-2 text-xs">
                    <Checkbox
                      id={`send-back-issue-${index}`}
                      checked={ticked.has(index)}
                      onCheckedChange={(checked) => toggle(index, checked === true)}
                    />
                    <label htmlFor={`send-back-issue-${index}`} className="min-w-0 flex-1">
                      <span className="text-muted-foreground">
                        {t(`chapter:review.severity.${issue.severity}`)} ·{" "}
                        {t(`chapter:review.category.${issue.category}`)} ·{" "}
                        {issue.pids.join(", ")}
                      </span>
                      {issue.quote === "" ? null : (
                        <span className="ml-1">“{issue.quote}”</span>
                      )}
                      <span className="ml-1">{issue.problem}</span>
                    </label>
                  </li>
                ))}
              </ul>
              <p className="text-xs text-muted-foreground">
                {t("chapter:send_back_dialog.selected_count", { count: selected.length })}
              </p>
            </div>
          )}

          <div className="space-y-1.5">
            <div className="text-xs font-medium">{t("chapter:send_back_dialog.mode_label")}</div>
            <RadioGroup
              value={mode}
              onValueChange={(value) => setMode(value as FeedbackMode)}
              className="gap-2"
            >
              <div className="flex items-start gap-2">
                <RadioGroupItem value="revise" id="send-back-mode-revise" className="mt-0.5" />
                <div>
                  <Label htmlFor="send-back-mode-revise" className="text-xs">
                    {t("chapter:send_back_dialog.mode_revise")}
                  </Label>
                  <p className="text-xs text-muted-foreground">
                    {t("chapter:send_back_dialog.mode_revise_hint")}
                  </p>
                </div>
              </div>
              <div className="flex items-start gap-2">
                <RadioGroupItem value="redraft" id="send-back-mode-redraft" className="mt-0.5" />
                <div>
                  <Label htmlFor="send-back-mode-redraft" className="text-xs">
                    {t("chapter:send_back_dialog.mode_redraft")}
                  </Label>
                  <p className="text-xs text-muted-foreground">
                    {t("chapter:send_back_dialog.mode_redraft_hint")}
                  </p>
                </div>
              </div>
            </RadioGroup>
          </div>
          <p className="text-xs text-muted-foreground">
            {t("chapter:send_back_dialog.version_note")}
          </p>
        </div>
      <DialogFooter>
        <Button variant="ghost" onClick={() => onOpenChange(false)}>
          {t("common:cancel")}
        </Button>
        <Button disabled={feedback.trim() === "" || sendBack.isPending} onClick={submit}>
          {t("chapter:send_back_dialog.confirm")}
        </Button>
      </DialogFooter>
    </>
  )
}
