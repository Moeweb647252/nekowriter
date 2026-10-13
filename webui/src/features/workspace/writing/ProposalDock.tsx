import type { SubmitEvent } from "react"
import { useTranslation } from "react-i18next"

import { ApprovalModeSwitch } from "./DraftingDock"
import { useProposalSummary } from "./edits"
import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { ProposalView } from "@/api/generated/ProposalView"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useDecideReview } from "@/api/queries/workspace"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Field, FieldDescription, FieldError, FieldLabel } from "@/components/ui/field"
import { Textarea } from "@/components/ui/textarea"
import { useErrorText } from "@/i18n/errors"
import { useDraft } from "@/lib/drafts"
import { submitOnModEnter } from "@/lib/keyboard"

/**
 * Reviewing one proposed edit (§9.5): empty feedback accepts it, any feedback rejects it,
 * and the button says which.
 */
export function ProposalDock({ view, proposal }: { view: WorkspaceView; proposal: ProposalView }) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const summary = useProposalSummary()
  const decide = useDecideReview(view.version.id)
  const feedback = useDraft(`${view.version.id}.review.${proposal.reviewId}`, "")
  const accepts = feedback.value.trim() === ""

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    decide.mutate(
      {
        revision: view.revision,
        reviewId: proposal.reviewId,
        decision: accepts ? { kind: "approve" } : { kind: "reject", feedback: feedback.value },
      },
      { onSuccess: feedback.clear },
    )
  }

  const fieldError = errorForField(decide.error, "review_feedback")
  const formError = errorForForm(decide.error, ["review_feedback"])
  const id = `proposal-feedback-${proposal.reviewId}`
  return (
    <div className="grid gap-6 md:grid-cols-2">
      <div className="flex flex-col items-start gap-3">
        <p role="status" className="font-medium">
          {summary(proposal.edit)}
        </p>
        <ApprovalModeSwitch view={view} />
      </div>
      <form onSubmit={submit} className="flex flex-col gap-3">
        <Field data-invalid={fieldError !== undefined}>
          <FieldLabel htmlFor={id}>{t(($) => $.workspace.proposal.feedback)}</FieldLabel>
          <Textarea
            id={id}
            value={feedback.value}
            onChange={(event) => feedback.setValue(event.target.value)}
            onKeyDown={submitOnModEnter}
            aria-invalid={fieldError !== undefined}
            rows={2}
          />
          <FieldDescription>
            {feedback.unsaved ? t(($) => $.workspace.draft) : t(($) => $.workspace.proposal.hint)}
          </FieldDescription>
          {fieldError !== undefined && <FieldError>{errorText(fieldError)}</FieldError>}
        </Field>
        {formError !== undefined && <ErrorAlert error={formError} />}
        <div>
          <Button
            type="submit"
            disabled={decide.isPending}
            variant={accepts ? "default" : "outline"}
          >
            {accepts
              ? t(($) => $.workspace.proposal.accept)
              : t(($) => $.workspace.proposal.reject)}
          </Button>
        </div>
      </form>
    </div>
  )
}
