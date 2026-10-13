import type { SubmitEvent } from "react"
import { useTranslation } from "react-i18next"

import type { DocumentKey } from "../documents"
import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { ArtifactKind } from "@/api/generated/ArtifactKind"
import type { ArtifactView } from "@/api/generated/ArtifactView"
import type { ReviewView } from "@/api/generated/ReviewView"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useDecideReview } from "@/api/queries/workspace"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Field, FieldDescription, FieldError, FieldLabel } from "@/components/ui/field"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { useErrorText } from "@/i18n/errors"
import { useDraft } from "@/lib/drafts"
import { submitOnModEnter } from "@/lib/keyboard"
import { cn } from "@/lib/utils"

/**
 * Reviewing the preparation in four tabs (§18.3): each artifact is approved, or rejected
 * with feedback.
 */
export function ReviewDock({
  view,
  selected,
  onSelect,
}: {
  view: WorkspaceView
  selected: DocumentKey | null
  onSelect: (doc: DocumentKey) => void
}) {
  const { t } = useTranslation()
  return (
    <Tabs value={selected} onValueChange={(value: ArtifactKind) => onSelect(value)}>
      <TabsList aria-label={t(($) => $.workspace.preparation.reviewTabs)}>
        {view.artifacts.map(({ kind, review }) => (
          <TabsTrigger key={kind} value={kind}>
            {t(($) => $.workspace.artifact[kind])}
            {review?.status === "open" && (
              <span aria-hidden className="size-1.5 rounded-full bg-primary" />
            )}
          </TabsTrigger>
        ))}
      </TabsList>
      {view.artifacts.map((artifact) => (
        <TabsContent key={artifact.kind} value={artifact.kind} className="pt-3">
          <ReviewPanel view={view} artifact={artifact} />
        </TabsContent>
      ))}
    </Tabs>
  )
}

function ReviewPanel({ view, artifact }: { view: WorkspaceView; artifact: ArtifactView }) {
  const { t } = useTranslation()
  const review = artifact.review
  if (review === null) return <Note>{t(($) => $.workspace.preparation.waiting)}</Note>
  switch (review.status) {
    case "open":
      return <ReviewForm key={review.id} view={view} kind={artifact.kind} review={review} />
    case "approved":
      return <Note>{t(($) => $.workspace.preparation.decided)}</Note>
    case "rejected":
      return (
        <div className="flex flex-col gap-2">
          <Note>
            {t(($) => $.workspace.preparation.rejectedNote, { feedback: review.feedback ?? "" })}
          </Note>
          <Note>{t(($) => $.workspace.preparation.decided)}</Note>
        </div>
      )
    case "stale":
      return <Note>{t(($) => $.workspace.preparation.stale)}</Note>
  }
}

function Note({ children }: { children: string }) {
  return <p className="whitespace-pre-wrap text-muted-foreground">{children}</p>
}

function ReviewForm({
  view,
  kind,
  review,
}: {
  view: WorkspaceView
  kind: ArtifactKind
  review: ReviewView
}) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const decide = useDecideReview(view.version.id)
  const feedback = useDraft(`${view.version.id}.review.${review.id}`, "")
  const documentName = t(($) => $.workspace.artifact[kind])

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
          {feedback.unsaved
            ? t(($) => $.workspace.draft)
            : t(($) => $.workspace.preparation.feedbackHint)}
        </FieldDescription>
        {feedbackError !== undefined && <FieldError>{errorText(feedbackError)}</FieldError>}
      </Field>
      {formError !== undefined && <ErrorAlert error={formError} />}
      <div className="flex flex-wrap gap-2">
        <Button type="button" disabled={decide.isPending} onClick={approve}>
          {t(($) => $.workspace.preparation.approve, { document: documentName })}
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
