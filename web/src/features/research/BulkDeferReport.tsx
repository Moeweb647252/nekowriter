/**
 * The visible half of a bulk defer that did not fully succeed (D-139).
 *
 * A batch of eight per-task requests can fail part-way, and a toast is transient: the author may
 * look away, and the list underneath would then show some rows moved and some not with no
 * explanation. Both surfaces (the list and the overview card) render this component from the
 * outcome the shared hook returns, so the wording, the tone and the placement cannot drift.
 *
 * The two sentences are two keys, each rendered on its own line: the count comes from one
 * interpolation and the topics from another, never concatenated into a third sentence
 * (AGENTS.md §6).
 */
import type { BulkDeferOutcome } from "@/features/research/useBulkDefer"

export function BulkDeferReport({ outcome }: { outcome: BulkDeferOutcome | null }) {
  // A batch that moved every task is visible where it happened — the rows change to `deferred`
  // (the list) or the next action replaces the card (the overview) — so the report only speaks
  // when something did not move.
  if (outcome === null || outcome.failed.length === 0) {
    return null
  }
  return (
    <div role="alert" className="space-y-0.5 text-xs text-status-failed">
      <p>{outcome.message}</p>
      {outcome.failedDetail === "" ? null : <p>{outcome.failedDetail}</p>}
    </div>
  )
}
