import { useTranslation } from "react-i18next"

import type { DocumentKey } from "./documents"
import type { ChapterPlan } from "@/api/generated/ChapterPlan"
import type { ReviewStatus } from "@/api/generated/ReviewStatus"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { Badge } from "@/components/ui/badge"

/** Committed text in the reading face (§18.2). Only committed revisions are shown here. */
export function ReadingText({ text }: { text: string }) {
  return <div className="font-reading text-lg leading-[1.85] whitespace-pre-wrap">{text}</div>
}

export function ReviewBadge({ status }: { status: ReviewStatus }) {
  const { t } = useTranslation()
  const variant = {
    open: "default",
    approved: "secondary",
    rejected: "destructive",
    stale: "outline",
  } as const satisfies Record<ReviewStatus, "default" | "secondary" | "destructive" | "outline">
  return <Badge variant={variant[status]}>{t(($) => $.workspace.reviewStatus[status])}</Badge>
}

/** The chapters planned in the outline, in order. */
export function ChapterPlanList({
  plan,
}: {
  plan: readonly { title?: string | undefined; goal?: string | undefined }[]
}) {
  const { t } = useTranslation()
  return (
    <section className="mt-10">
      <h3 className="mb-3 font-heading text-lg font-semibold">
        {t(($) => $.workspace.document.chapterPlan)}
      </h3>
      <ol className="flex flex-col gap-3">
        {plan.map((item, index) => (
          // A streaming plan has no IDs yet; a plan's order is its identity.
          <li key={index}>
            <p className="font-medium">
              {t(($) => $.workspace.document.planItem, {
                number: index + 1,
                title: item.title ?? "",
              })}
            </p>
            {item.goal !== undefined && <p className="text-muted-foreground">{item.goal}</p>}
          </li>
        ))}
      </ol>
    </section>
  )
}

/** A preparation document as committed: the brief, or an artifact with its review. */
export function DocumentView({ view, doc }: { view: WorkspaceView; doc: DocumentKey }) {
  const { t } = useTranslation()

  if (doc === "brief") {
    return (
      <article>
        <header className="mb-6 flex flex-wrap items-baseline gap-3">
          <h2 className="font-heading text-2xl font-semibold">
            {t(($) => $.workspace.brief.label)}
          </h2>
        </header>
        {view.brief === null ? (
          <p className="text-muted-foreground">{t(($) => $.workspace.brief.empty)}</p>
        ) : (
          <ReadingText text={view.brief.content} />
        )}
      </article>
    )
  }

  const artifact = view.artifacts.find((item) => item.kind === doc)
  const plan: ChapterPlan | null = artifact?.chapterPlan ?? null
  return (
    <article>
      <header className="mb-6 flex flex-wrap items-baseline gap-3">
        <h2 className="font-heading text-2xl font-semibold">
          {t(($) => $.workspace.artifact[doc])}
        </h2>
        {artifact?.revision && (
          <span className="text-sm text-muted-foreground">
            {t(($) => $.workspace.document.revision, { number: artifact.revision.number })}
          </span>
        )}
        {artifact?.review && <ReviewBadge status={artifact.review.status} />}
      </header>
      {artifact?.revision ? (
        <ReadingText text={artifact.revision.content} />
      ) : (
        <p className="text-muted-foreground">{t(($) => $.workspace.document.notWritten)}</p>
      )}
      {plan !== null && <ChapterPlanList plan={plan} />}
    </article>
  )
}
