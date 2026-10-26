/**
 * The `request_review` form (PROJECT §8.7, spec/11 §5.3): per item a verdict toggle (Approve /
 * Needs changes) plus a feedback box, a general comment, and an explicit "Approve all" shortcut.
 *
 * The payload is exactly what spec/05 §7 documents:
 * `{docs: {outline?, world?, protagonist?, characters?, research_topics?: {approved, feedback?}},
 * general?}`. Item labels are the studio's tab names and come from i18n; the payload keys are the
 * protocol's enum values.
 *
 * **A verdict starts `unset` (D-134).** The old form initialised every item to `approved: true`,
 * so it opened pre-answered and clicking 「通过」 on an untouched row changed neither the markup nor
 * the payload — the button looked dead (the author's report: 「通过点了没用」). "Not reviewed yet" is
 * a real third state here: the payload's `approved` is a per-item bool the author is answering, and
 * the form must not answer it for them. Every click therefore moves a row from one visible state to
 * another, and the review cannot be submitted while any item is still unset.
 */
import { Check, CheckCheck, CircleDashed, PencilLine } from "lucide-react"
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

/**
 * One item's verdict in the form. `approved: null` is the third state: the author has not looked at
 * the item yet. Only `true` / `false` reach the payload (spec/05 §7).
 */
export type ReviewVerdictState = { approved: boolean | null; feedback: string }

/** The state a freshly opened form starts in: every item unset, i.e. nothing answered yet. */
export function initialReviewState(items: readonly string[]): Record<string, ReviewVerdictState> {
  return Object.fromEntries(items.map((item) => [item, { approved: null, feedback: "" }]))
}

/** The verdict currently recorded for one item (an item the form has not seen yet is unset). */
export function verdictOf(
  state: Record<string, ReviewVerdictState>,
  item: string,
): ReviewVerdictState {
  return state[item] ?? { approved: null, feedback: "" }
}

/**
 * Records one explicit verdict. The feedback draft is kept: a stray click on the other verdict and
 * back must not throw away what the author typed, and the box is hidden — not destroyed — in the
 * approved state. The payload leaves the draft out for an approved item (see `buildReviewPayload`),
 * so the kept text can never contradict the verdict the author submitted.
 */
export function withVerdict(
  state: Record<string, ReviewVerdictState>,
  item: string,
  approved: boolean,
): Record<string, ReviewVerdictState> {
  const current = verdictOf(state, item)
  return { ...state, [item]: { approved, feedback: current.feedback } }
}

/**
 * One click on a verdict button: it selects that verdict, or — when it is already selected — clears
 * the item back to `unset`.
 *
 * The toggle is what keeps *every* click visible. `aria-pressed` on the buttons is the ARIA
 * toggle-button pattern, where activating a pressed button releases it; and the release is also the
 * only way back to "not reviewed yet" if the author changes their mind (`Approve all` otherwise
 * leaves no per-row way to undo). Without it, clicking an already-selected verdict would again be a
 * silent no-op, which is the defect this form is fixing (D-134).
 */
export function toggleVerdict(
  state: Record<string, ReviewVerdictState>,
  item: string,
  approved: boolean,
): Record<string, ReviewVerdictState> {
  if (verdictOf(state, item).approved === approved) {
    return { ...state, [item]: { approved: null, feedback: "" } }
  }
  return withVerdict(state, item, approved)
}

/** The "Approve all" shortcut: an explicit action instead of a pre-set default. */
export function approveAllItems(
  state: Record<string, ReviewVerdictState>,
  items: readonly string[],
): Record<string, ReviewVerdictState> {
  const next = { ...state }
  for (const item of items) {
    next[item] = { ...verdictOf(state, item), approved: true }
  }
  return next
}

/** The items still waiting for a verdict, in tab order. */
export function unreviewedItems(
  state: Record<string, ReviewVerdictState>,
  items: readonly string[],
): string[] {
  return items.filter((item) => verdictOf(state, item).approved === null)
}

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
    // An unreviewed item is omitted: the payload's items are optional (spec/05 §7), and the form
    // refuses to submit while any of them is unset.
    if (value.approved === null) {
      continue
    }
    // Feedback accompanies a rejection: an approved item's box is hidden, so any draft left in it is
    // stale and must not be sent as if it were a condition of the approval.
    const feedback = value.approved ? "" : value.feedback.trim()
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
    initialReviewState(items),
  )
  const [general, setGeneral] = useState("")

  const decide = (item: string, approved: boolean) =>
    setState((previous) => toggleVerdict(previous, item, approved))

  const remaining = unreviewedItems(state, items)
  const allApproved = items.every((item) => verdictOf(state, item).approved === true)

  return (
    <div className="space-y-3 rounded-md border border-sky-500/40 bg-sky-500/5 p-3">
      <div className="flex items-start justify-between gap-2">
        {message.trim() === "" ? (
          <span className="flex-1" />
        ) : (
          <p className="flex-1 text-sm text-muted-foreground">{message}</p>
        )}
        <Button
          size="xs"
          variant="outline"
          className="shrink-0"
          disabled={pending || allApproved}
          onClick={() => setState((previous) => approveAllItems(previous, items))}
        >
          <CheckCheck aria-hidden="true" />
          {t("book:conception.approve_all")}
        </Button>
      </div>
      {items.map((item) => {
        const value = verdictOf(state, item)
        return (
          <div
            key={item}
            data-review-item={item}
            data-verdict={
              value.approved === null ? "unset" : value.approved ? "approved" : "needs_changes"
            }
            className={cn(
              "space-y-1.5 rounded border bg-card p-2",
              value.approved === null && "border-dashed",
            )}
          >
            <div className="flex items-center justify-between gap-2">
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-sm font-medium">{t(`book:conception.item.${item}`)}</span>
                <span
                  data-verdict-pill={item}
                  className={cn(
                    "flex items-center gap-1 rounded-full border px-1.5 py-0.5 text-xs",
                    value.approved === null
                      ? "border-dashed text-muted-foreground"
                      : value.approved
                        ? "border-primary/40 bg-primary/10 text-primary"
                        : "border-amber-600/40 bg-amber-600/10 text-amber-600",
                  )}
                >
                  {value.approved === null ? (
                    <CircleDashed aria-hidden="true" className="size-3" />
                  ) : value.approved ? (
                    <Check aria-hidden="true" className="size-3" />
                  ) : (
                    <PencilLine aria-hidden="true" className="size-3" />
                  )}
                  {value.approved === null
                    ? t("book:conception.verdict.unset")
                    : value.approved
                      ? t("book:conception.verdict.approved")
                      : t("book:conception.verdict.needs_changes")}
                </span>
              </div>
              <div className="flex gap-1">
                <Button
                  size="xs"
                  variant={value.approved === true ? "default" : "outline"}
                  aria-pressed={value.approved === true}
                  onClick={() => decide(item, true)}
                >
                  {value.approved === true ? <Check aria-hidden="true" /> : null}
                  {t("book:conception.approve")}
                </Button>
                <Button
                  size="xs"
                  variant={value.approved === false ? "default" : "outline"}
                  className={cn(
                    value.approved === false && "bg-amber-600 hover:bg-amber-600/90",
                  )}
                  aria-pressed={value.approved === false}
                  onClick={() => decide(item, false)}
                >
                  {value.approved === false ? <PencilLine aria-hidden="true" /> : null}
                  {t("book:conception.needs_changes")}
                </Button>
              </div>
            </div>
            {value.approved === false ? (
              // `aria-label` names the box for assistive tech: the placeholder is not a label, and
              // the general comment box is otherwise indistinguishable from it.
              <Textarea
                rows={2}
                aria-label={t("book:conception.feedback_for", {
                  item: t(`book:conception.item.${item}`),
                })}
                value={value.feedback}
                placeholder={t("book:conception.feedback_placeholder")}
                onChange={(event) =>
                  setState((previous) => ({
                    ...previous,
                    [item]: { ...verdictOf(previous, item), feedback: event.target.value },
                  }))
                }
              />
            ) : null}
          </div>
        )
      })}
      <div className="space-y-1">
        <Textarea
          rows={2}
          aria-label={t("book:conception.general_comment")}
          value={general}
          onChange={(event) => setGeneral(event.target.value)}
        />
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {remaining.length === 0 ? null : (
          // A disabled button with no explanation is its own usability defect: say why, and say how
          // many items are left (the same class of bug as the dead Approve button).
          <p role="status" className="flex-1 text-xs text-amber-600">
            {remaining.length === 1
              ? t("book:conception.review_incomplete_one", { count: remaining.length })
              : t("book:conception.review_incomplete_other", { count: remaining.length })}
          </p>
        )}
        <Button
          size="sm"
          className={remaining.length === 0 ? "ml-auto" : undefined}
          disabled={pending || remaining.length > 0}
          onClick={() => onSubmit(buildReviewPayload(state, general))}
        >
          {pending ? t("common:saving") : t("book:conception.submit_review")}
        </Button>
      </div>
    </div>
  )
}
