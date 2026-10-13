import type { SubmitEvent } from "react"
import { useTranslation } from "react-i18next"

import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { Citation } from "@/api/generated/Citation"
import type { ReviewView } from "@/api/generated/ReviewView"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useDecideReview, useDelegateReview } from "@/api/queries/workspace"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Field, FieldDescription, FieldError, FieldLabel } from "@/components/ui/field"
import { Textarea } from "@/components/ui/textarea"
import { useErrorText } from "@/i18n/errors"
import { useDraft, useDraftList } from "@/lib/drafts"
import { submitOnModEnter } from "@/lib/keyboard"
import { cn } from "@/lib/utils"

import { CitationField } from "./CitationField"
import { DelegationFailure } from "./Delegation"

/**
 * Approving a submission, or rejecting it with feedback (§10). The two actions are separate
 * buttons; rejecting needs feedback or a citation, and the draft is kept in this browser.
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
  const delegate = useDelegateReview(view.version.id)
  const key = `${view.version.id}.review.${review.id}`
  const feedback = useDraft(key, "")
  const citations = useDraftList<Citation>(`${key}.citations`)
  const error = decide.error ?? delegate.error
  const pending = decide.isPending || delegate.isPending
  const clear = () => {
    feedback.clear()
    citations.clear()
  }

  const reject = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    decide.mutate(
      {
        revision: view.revision,
        reviewId: review.id,
        decision: { kind: "reject", feedback: feedback.value, citations: citations.items },
      },
      { onSuccess: clear },
    )
  }
  const approve = () => {
    decide.mutate(
      { revision: view.revision, reviewId: review.id, decision: { kind: "approve" } },
      { onSuccess: clear },
    )
  }

  const delegateIt = () => {
    delegate.mutate(
      {
        revision: view.revision,
        reviewId: review.id,
        notes: feedback.value,
        citations: citations.items,
      },
      { onSuccess: clear },
    )
  }

  const feedbackError = errorForField(error, "review_feedback")
  const formError = errorForForm(error, ["review_feedback"])
  const id = `feedback-${review.id}`

  return (
    <form onSubmit={reject} className="flex flex-col gap-3">
      <DelegationFailure review={review} />
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
      <CitationField citations={citations} />
      {formError !== undefined && <ErrorAlert error={formError} />}
      <div className="flex flex-wrap gap-2">
        <Button type="button" disabled={pending} onClick={approve}>
          {approveLabel}
        </Button>
        <Button
          type="submit"
          variant="outline"
          disabled={pending || (feedback.value.trim() === "" && citations.items.length === 0)}
        >
          {t(($) => $.workspace.preparation.requestChanges)}
        </Button>
        <Button
          type="button"
          variant="ghost"
          disabled={pending}
          onClick={delegateIt}
          title={t(($) => $.workspace.delegation.delegateHint)}
        >
          {t(($) => $.workspace.delegation.delegate)}
        </Button>
      </div>
    </form>
  )
}
