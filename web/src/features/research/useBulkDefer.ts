/**
 * Bulk defer: postpone every `ready` research task of one book at once (D-139).
 *
 * The author had eight `ready` tasks and the overview kept insisting they run research before
 * writing; the release valve D-138 added was eight separate clicks, and the card demanding the
 * research offered no way to say "not now" at all. This module is the **one** place that batches
 * them, and both surfaces use it — `ResearchTaskList` and the overview's `run_research` card —
 * so they cannot drift in scoping, sequencing, reporting or invalidation.
 *
 * **Sequencing**: the endpoints are per task and transactional, so the loop is strictly one
 * request at a time. Bounded concurrency would buy milliseconds for eight rows and cost the one
 * thing this needs — a failure that is attributable to one task while the rest keep going.
 * **Partial failure** is reported, never swallowed: the outcome names how many moved and which
 * tasks did not (by topic, not by id), the toast carries the same sentence, and the button renders
 * it inline so it survives the toast. `defer` is idempotent (a repeat returns the current row,
 * D-138), so a retry after a failure is safe.
 */
import { useCallback, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import type { ResearchTaskDto } from "@/api/gen/ResearchTaskDto"
import { useDeferResearchTask, useResearchTasks } from "@/api/hooks/research"

/** One task the batch could not postpone. */
export type BulkDeferFailure = {
  taskId: number
  /** The task's topic, which is what the author recognizes; ids are not shown to them. */
  topic: string
  error: unknown
}

/** What one run did, and the localized sentences that say it. */
export type BulkDeferOutcome = {
  /** The ids that moved, in list order. */
  deferred: number[]
  failed: BulkDeferFailure[]
  /** The one-line report, e.g. "Deferred 7 of 8 research tasks". */
  message: string
  /** `"Not deferred: topic, topic"`, or empty when every task moved. */
  failedDetail: string
}

const NOTHING_DEFERRED: BulkDeferOutcome = {
  deferred: [],
  failed: [],
  message: "",
  failedDetail: "",
}

/**
 * The batch operation. `bookId` is the scope: a book-scoped surface (the book's knowledge page,
 * the overview card) defers that book's tasks and nothing else; the global research page passes
 * nothing and defers every `ready` task it lists.
 */
export function useDeferReadyResearchTasks({ bookId }: { bookId?: number } = {}) {
  const { t } = useTranslation()
  // The same book-scoped list query the list renders, so the candidates are exactly the rows the
  // author is looking at (one cache entry shared with `ResearchTaskList`).
  const tasks = useResearchTasks({ bookId })
  // The single-task mutation, reused rather than reimplemented: it owns the invalidation of the
  // research queries *and* of that book's overview (D-138), which is what makes the card change.
  const defer = useDeferResearchTask()
  const [isPending, setIsPending] = useState(false)
  const inFlight = useRef<Promise<BulkDeferOutcome> | null>(null)

  // Two filters, both deliberate. `status === "ready"` is the only runnable state (a `draft` is
  // unconfirmed, a `deferred` task is already postponed, `done`/`running` cannot be deferred);
  // `book_id === bookId` is defence in depth — the server already filters by book, and this makes
  // a stale or wrongly shaped cache entry unable to postpone another book's work.
  const readyTasks = useMemo(() => {
    const rows: ResearchTaskDto[] = tasks.data ?? []
    return rows.filter(
      (task) => task.status === "ready" && (bookId === undefined || task.book_id === bookId),
    )
  }, [tasks.data, bookId])

  const report = useCallback(
    (deferred: number[], failed: BulkDeferFailure[], total: number): BulkDeferOutcome => {
      const failedDetail =
        failed.length === 0
          ? ""
          : t("knowledge:research.bulk_defer_failed_list", {
              topics: failed.map((failure) => failure.topic).join(", "),
            })
      if (failed.length === 0) {
        const message =
          deferred.length === 1
            ? t("knowledge:research.bulk_defer_done_one", { count: deferred.length })
            : t("knowledge:research.bulk_defer_done_other", { count: deferred.length })
        toast.success(message)
        return { deferred, failed, message, failedDetail }
      }
      // `count` is the number attempted, so the sentence's plural agrees with the whole batch
      // ("Deferred 2 of 3 research tasks" / "Deferred 0 of 1 research task") while `deferred`
      // carries the number that actually moved.
      const message =
        total === 1
          ? t("knowledge:research.bulk_defer_partial_one", { count: total, deferred: deferred.length })
          : t("knowledge:research.bulk_defer_partial_other", {
              count: total,
              deferred: deferred.length,
            })
      if (deferred.length === 0) {
        toast.error(message, { description: failedDetail })
      } else {
        toast.warning(message, { description: failedDetail })
      }
      return { deferred, failed, message, failedDetail }
    },
    [t],
  )

  const run = useCallback((): Promise<BulkDeferOutcome> => {
    // A double click cannot start a second batch: the promise is shared, so both callers see one
    // run and one report (the requests are idempotent, but two reports would be a lie).
    if (inFlight.current !== null) {
      return inFlight.current
    }
    // Snapshot the targets: each success invalidates the list, so the refetched rows shrink while
    // the loop is still running. Iterating the live list would skip rows; the snapshot defers
    // exactly what was on screen when the author clicked.
    const targets = readyTasks.map((task) => ({ id: task.id, topic: task.topic }))
    if (targets.length === 0) {
      return Promise.resolve(NOTHING_DEFERRED)
    }
    const promise = (async () => {
      setIsPending(true)
      const deferred: number[] = []
      const failed: BulkDeferFailure[] = []
      try {
        for (const target of targets) {
          try {
            await defer.mutateAsync(target.id)
            deferred.push(target.id)
          } catch (error) {
            // One failure does not abort the batch: the author asked for all of them, and a
            // half-finished batch is more useful than a batch that stopped at the first refusal.
            failed.push({ taskId: target.id, topic: target.topic, error })
          }
        }
      } finally {
        setIsPending(false)
      }
      return report(deferred, failed, targets.length)
    })()
    inFlight.current = promise
    void promise.finally(() => {
      inFlight.current = null
    })
    return promise
  }, [defer, readyTasks, report])

  return { readyTasks, run, isPending, isLoading: tasks.isPending }
}
