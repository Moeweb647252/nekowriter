import { useTranslation } from "react-i18next"

import type { ReviewView } from "@/api/generated/ReviewView"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useCancelDelegation } from "@/api/queries/workspace"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { useFormatters } from "@/i18n/format"

/** A review the review agent is deciding, which the user can take back (§10). */
export function DelegatedReview({ view, review }: { view: WorkspaceView; review: ReviewView }) {
  const { t } = useTranslation()
  const cancel = useCancelDelegation(view.version.id)
  return (
    <div className="flex flex-col items-start gap-3">
      <p role="status" className="flex items-center gap-2">
        <Spinner />
        {t(($) => $.workspace.delegation.delegated)}
      </p>
      {cancel.isError && <ErrorAlert error={cancel.error} />}
      <Button
        variant="outline"
        disabled={cancel.isPending}
        onClick={() => cancel.mutate(review.id)}
      >
        {t(($) => $.workspace.delegation.cancel)}
      </Button>
    </div>
  )
}

/** Why the last delegation of an open review failed. */
export function DelegationFailure({ review }: { review: ReviewView }) {
  const { t } = useTranslation()
  const failure = review.delegationFailure
  if (failure === null) return null
  return (
    <p role="alert" className="text-sm text-destructive">
      {t(($) => $.workspace.delegation.failed, {
        reason: t(($) => $.workspace.delegation.reason[failure.kind]),
      })}
    </p>
  )
}

/** Who decided a review, when a review agent did, with its confidence. */
export function DecidedBy({ review }: { review: ReviewView }) {
  const { t } = useTranslation()
  const format = useFormatters()
  if (review.decidedBy !== "delegate" || review.confidencePermille === null) return null
  return (
    <p className="text-sm text-muted-foreground">
      {t(($) => $.workspace.delegation.decided, {
        confidence: format.percent(review.confidencePermille / 1000),
      })}
    </p>
  )
}
