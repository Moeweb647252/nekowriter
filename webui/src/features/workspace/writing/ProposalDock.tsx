import type { SubmitEvent } from "react"
import { useTranslation } from "react-i18next"

import { CitationField } from "../CitationField"
import { DelegatedReview, DelegationFailure } from "../Delegation"
import { ApprovalModeSwitch } from "./DraftingDock"
import { useProposalSummary } from "./edits"
import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { Citation } from "@/api/generated/Citation"
import type { ProposalView } from "@/api/generated/ProposalView"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useDecideReview, useDelegateReview } from "@/api/queries/workspace"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Field, FieldDescription, FieldError, FieldLabel } from "@/components/ui/field"
import { Textarea } from "@/components/ui/textarea"
import { useErrorText } from "@/i18n/errors"
import { useDraft, useDraftList } from "@/lib/drafts"
import { submitOnModEnter } from "@/lib/keyboard"

/**
 * Reviewing one proposed edit (§9.5): empty feedback without citations accepts it, any
 * feedback or citation rejects it, and the button says which. The review can also go to
 * the review agent (§10).
 */
export function ProposalDock({ view, proposal }: { view: WorkspaceView; proposal: ProposalView }) {
  const summary = useProposalSummary()
  return (
    <div className="grid gap-6 md:grid-cols-2">
      <div className="flex flex-col items-start gap-3">
        <p role="status" className="font-medium">
          {summary(proposal.edit)}
        </p>
        <ApprovalModeSwitch view={view} />
      </div>
      {proposal.review.status === "delegated" ? (
        <DelegatedReview view={view} review={proposal.review} />
      ) : (
        <ProposalForm view={view} proposal={proposal} />
      )}
    </div>
  )
}

function ProposalForm({ view, proposal }: { view: WorkspaceView; proposal: ProposalView }) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const decide = useDecideReview(view.version.id)
  const delegate = useDelegateReview(view.version.id)
  const review = proposal.review
  const key = `${view.version.id}.review.${review.id}`
  const feedback = useDraft(key, "")
  const citations = useDraftList<Citation>(`${key}.citations`)
  const accepts = feedback.value.trim() === "" && citations.items.length === 0
  const error = decide.error ?? delegate.error
  const pending = decide.isPending || delegate.isPending
  const clear = () => {
    feedback.clear()
    citations.clear()
  }

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    decide.mutate(
      {
        revision: view.revision,
        reviewId: review.id,
        decision: accepts
          ? { kind: "approve" }
          : { kind: "reject", feedback: feedback.value, citations: citations.items },
      },
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

  const fieldError = errorForField(error, "review_feedback")
  const formError = errorForForm(error, ["review_feedback"])
  const id = `proposal-feedback-${review.id}`
  return (
    <form onSubmit={submit} className="flex flex-col gap-3">
      <DelegationFailure review={review} />
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
      <CitationField citations={citations} />
      {formError !== undefined && <ErrorAlert error={formError} />}
      <div className="flex flex-wrap gap-2">
        <Button type="submit" disabled={pending} variant={accepts ? "default" : "outline"}>
          {accepts ? t(($) => $.workspace.proposal.accept) : t(($) => $.workspace.proposal.reject)}
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
