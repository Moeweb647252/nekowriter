import { useTranslation } from "react-i18next"

import { CitedQuotes } from "../CitationField"
import type { DocumentKey } from "../documents"
import { DecidedBy, DelegatedReview } from "../Delegation"
import { ReviewForm } from "../ReviewForm"
import type { ArtifactKind } from "@/api/generated/ArtifactKind"
import type { ArtifactView } from "@/api/generated/ArtifactView"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"

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
      return (
        <ReviewForm
          key={review.id}
          view={view}
          review={review}
          approveLabel={t(($) => $.workspace.preparation.approve, {
            document: t(($) => $.workspace.artifact[artifact.kind]),
          })}
          hint={t(($) => $.workspace.preparation.feedbackHint)}
        />
      )
    case "delegated":
      return <DelegatedReview view={view} review={review} />
    case "approved":
      return (
        <div className="flex flex-col gap-2">
          <Note>{t(($) => $.workspace.preparation.decided)}</Note>
          <DecidedBy review={review} />
        </div>
      )
    case "rejected":
      return (
        <div className="flex flex-col gap-2">
          {review.feedback !== null && (
            <Note>
              {t(($) => $.workspace.preparation.rejectedNote, { feedback: review.feedback })}
            </Note>
          )}
          <CitedQuotes citations={review.citations} />
          <Note>{t(($) => $.workspace.preparation.decided)}</Note>
          <DecidedBy review={review} />
        </div>
      )
    case "stale":
      return <Note>{t(($) => $.workspace.preparation.stale)}</Note>
  }
}

function Note({ children }: { children: string }) {
  return <p className="whitespace-pre-wrap text-muted-foreground">{children}</p>
}
