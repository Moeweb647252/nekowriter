import type { SubmitEvent } from "react"
import { useTranslation } from "react-i18next"

import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { ReviewView } from "@/api/generated/ReviewView"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useDecideReview } from "@/api/queries/workspace"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Field, FieldDescription, FieldError, FieldLabel } from "@/components/ui/field"
import { Textarea } from "@/components/ui/textarea"
import { useErrorText } from "@/i18n/errors"
import { useDraft } from "@/lib/drafts"
import { submitOnModEnter } from "@/lib/keyboard"
import { cn } from "@/lib/utils"

/**
 * Approving a submission, or rejecting it with feedback (§10). The two actions are separate
 * buttons; rejecting needs feedback, and the feedback draft is kept in this browser.
 */
export function ReviewForm({
  view,
  review,
  approveLabel,
  hint,
}: {
  view: WorkspaceView
  review: ReviewView
  approveLabel: string
  hint: string
}) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const decide = useDecideReview(view.version.id)
  const feedback = useDraft(`${view.version.id}.review.${review.id}`, "")

  const reject = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    decide.mutate(
      {
        revision: view.revision,
        reviewId: review.id,
        decision: { kind: "reject", feedback: feedback.value },
      },
      { onSuccess: feedback.clear },
    )
  }
  const approve = () => {
    decide.mutate(
      { revision: view.revision, reviewId: review.id, decision: { kind: "approve" } },
      { onSuccess: feedback.clear },
    )
  }

  const feedbackError = errorForField(decide.error, "review_feedback")
  const formError = errorForForm(decide.error, ["review_feedback"])
  const id = `feedback-${review.id}`

  return (
    <form onSubmit={reject} className="flex flex-col gap-3">
      <Field data-invalid={feedbackError !== undefined}>
        <FieldLabel htmlFor={id}>{t(($) => $.workspace.preparation.feedback)}</FieldLabel>
        <Textarea
          id={id}
          value={feedback.value}
          onChange={(event) => feedback.setValue(event.target.value)}
          onKeyDown={submitOnModEnter}
          aria-invalid={feedbackError !== undefined}
          rows={3}
        />
        <FieldDescription className={cn(feedback.unsaved && "text-foreground")}>
          {feedback.unsaved ? t(($) => $.workspace.draft) : hint}
        </FieldDescription>
        {feedbackError !== undefined && <FieldError>{errorText(feedbackError)}</FieldError>}
      </Field>
      {formError !== undefined && <ErrorAlert error={formError} />}
      <div className="flex flex-wrap gap-2">
        <Button type="button" disabled={decide.isPending} onClick={approve}>
          {approveLabel}
        </Button>
        <Button
          type="submit"
          variant="outline"
          disabled={decide.isPending || feedback.value.trim() === ""}
        >
          {t(($) => $.workspace.preparation.requestChanges)}
        </Button>
      </div>
    </form>
  )
}
