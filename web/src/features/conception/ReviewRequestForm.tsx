/**
 * The `request_review` form (PROJECT §8.7, spec/11 §5.3): per item an Approve / Needs changes
 * toggle plus a feedback box, and a general comment.
 *
 * The payload is exactly what spec/05 §7 documents:
 * `{docs: {outline?, world?, protagonist?, characters?, research_topics?: {approved, feedback?}},
 * general?}`. Item labels are the studio's tab names and come from i18n; the payload keys are the
 * protocol's enum values.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { cn } from "@/lib/utils"

/** The reviewable items, in the studio's tab order (spec/05 §7). */
export const REVIEW_ITEMS = [
  "outline",
  "characters",
  "world",
  "protagonist",
  "research_topics",
] as const
export type ReviewItem = (typeof REVIEW_ITEMS)[number]

/** One item's verdict in the form. */
export type ReviewVerdictState = { approved: boolean; feedback: string }

/** The answer payload of the form, as `POST /sessions/:id/answer` expects it (spec/05 §7). */
export function buildReviewPayload(
  state: Record<string, ReviewVerdictState>,
  general: string,
): {
  docs: Record<string, { approved: boolean; feedback?: string }>
  general?: string
} {
  const docs: Record<string, { approved: boolean; feedback?: string }> = {}
  for (const [key, value] of Object.entries(state)) {
    const feedback = value.feedback.trim()
    docs[key] = {
      approved: value.approved,
      ...(feedback === "" ? {} : { feedback }),
    }
  }
  const comment = general.trim()
  return { docs, ...(comment === "" ? {} : { general: comment }) }
}

export function ReviewRequestForm({
  message,
  items,
  pending,
  onSubmit,
}: {
  /** The agent's note about what to look at (`ui.message`). */
  message: string
  /** The items the studio can show (the ones present in the book). */
  items: ReviewItem[]
  pending: boolean
  onSubmit: (payload: ReturnType<typeof buildReviewPayload>) => void
}) {
  const { t } = useTranslation()
  const [state, setState] = useState<Record<string, ReviewVerdictState>>(() =>
    Object.fromEntries(items.map((item) => [item, { approved: true, feedback: "" }])),
  )
  const [general, setGeneral] = useState("")

  const setItem = (item: string, patch: Partial<ReviewVerdictState>) =>
    setState((previous) => ({
      ...previous,
      [item]: { ...previous[item], ...patch },
    }))

  return (
    <div className="space-y-3 rounded-md border border-sky-500/40 bg-sky-500/5 p-3">
      {message.trim() === "" ? null : (
        <p className="text-sm text-muted-foreground">{message}</p>
      )}
      {items.map((item) => {
        const value = state[item] ?? { approved: true, feedback: "" }
        return (
          <div key={item} className="space-y-1.5 rounded border bg-card p-2">
            <div className="flex items-center justify-between gap-2">
              <span className="text-sm font-medium">{t(`book:conception.item.${item}`)}</span>
              <div className="flex gap-1">
                <Button
                  size="xs"
                  variant={value.approved ? "default" : "outline"}
                  onClick={() => setItem(item, { approved: true })}
                >
                  {t("book:conception.approve")}
                </Button>
                <Button
                  size="xs"
                  variant={value.approved ? "outline" : "default"}
                  className={cn(!value.approved && "bg-amber-600 hover:bg-amber-600/90")}
                  onClick={() => setItem(item, { approved: false })}
                >
                  {t("book:conception.needs_changes")}
                </Button>
              </div>
            </div>
            {value.approved ? null : (
              <Textarea
                rows={2}
                value={value.feedback}
                placeholder={t("book:conception.feedback_placeholder")}
                onChange={(event) => setItem(item, { feedback: event.target.value })}
              />
            )}
          </div>
        )
      })}
      <div className="space-y-1">
        <span className="text-xs text-muted-foreground">
          {t("book:conception.general_comment")}
        </span>
        <Textarea
          rows={2}
          value={general}
          onChange={(event) => setGeneral(event.target.value)}
        />
      </div>
      <Button
        size="sm"
        disabled={pending}
        onClick={() => onSubmit(buildReviewPayload(state, general))}
      >
        {pending ? t("common:saving") : t("book:conception.submit_review")}
      </Button>
    </div>
  )
}
