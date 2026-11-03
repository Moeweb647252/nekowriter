/** The research task list (PROJECT §8.10): the tasks of one book, or every task. */
import { useState } from "react"
import { Link } from "react-router"
import { useTranslation } from "react-i18next"
import { CalendarClock, FlaskConical, Loader2, Plus, Undo2 } from "lucide-react"

import { useDeferResearchTask, useResearchTasks, useResumeResearchTask } from "@/api/hooks/research"
import { EmptyState } from "@/components/common/EmptyState"
import { useErrorToast } from "@/components/common/ErrorToast"
import { StatusBadge } from "@/components/common/StatusBadge"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { BulkDeferReport } from "@/features/research/BulkDeferReport"
import { useDeferReadyResearchTasks, type BulkDeferOutcome } from "@/features/research/useBulkDefer"

/** A compact list of research tasks, used by the book knowledge page and `/research`. */
export function ResearchTaskList({
  bookId,
  onCreate,
}: {
  bookId?: number
  onCreate?: () => void
}) {
  const { t } = useTranslation()
  const tasks = useResearchTasks({ bookId })
  // The batch shares this list's book-scoped query, so "Defer all (n)" names exactly the `ready`
  // rows below and defers nothing else (D-139). Both surfaces use this one hook.
  const bulk = useDeferReadyResearchTasks({ bookId })
  const [outcome, setOutcome] = useState<BulkDeferOutcome | null>(null)
  // Deferring is also a per-row action here because the author typically has a handful of runnable
  // tasks at once and would otherwise open one page per task to postpone them (D-138).
  const defer = useDeferResearchTask()
  const resumeTask = useResumeResearchTask()
  const showError = useErrorToast()

  if (tasks.isPending) {
    return <Skeleton className="h-24" />
  }
  const items = tasks.data ?? []
  const readyCount = bulk.readyTasks.length

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-sm font-medium">{t("knowledge:research.list_title")}</h3>
        <div className="flex items-center gap-2">
          {readyCount === 0 ? null : (
            // Hidden when nothing is runnable: a disabled "Defer all (0)" would be a control that
            // can never do anything. The count is in the label, so the scope of the click is
            // visible before it happens.
            <Button
              size="sm"
              variant="outline"
              disabled={bulk.isPending}
              onClick={() => void bulk.run().then(setOutcome)}
            >
              {bulk.isPending ? (
                <Loader2 className="animate-spin" aria-hidden="true" />
              ) : (
                <CalendarClock aria-hidden="true" />
              )}
              {t("knowledge:research.defer_all", { count: readyCount })}
            </Button>
          )}
          {onCreate === undefined ? null : (
            <Button size="sm" variant="outline" onClick={onCreate}>
              <Plus className="size-3.5" aria-hidden="true" />
              {t("knowledge:research.new_task")}
            </Button>
          )}
        </div>
      </div>
      {readyCount === 0 ? null : (
        <p className="text-xs text-muted-foreground">{t("knowledge:research.defer_all_hint")}</p>
      )}
      {/* A partial batch stays visible after the toast has gone (D-139). */}
      <BulkDeferReport outcome={outcome} />
      {items.length === 0 ? (
        <EmptyState title={t("knowledge:research.empty")} description={t("knowledge:research.empty_hint")} />
      ) : (
        <ul className="divide-y rounded-md border">
          {items.map((task) => (
            <li key={task.id}>
              {/* The row is not one big link: the action button sits beside the link, so
                  clicking "defer" postpones the task instead of opening its page. */}
              <div className="flex items-center gap-3 px-3 py-2 hover:bg-accent">
                <Link
                  to={`/research/${task.id}`}
                  className="flex min-w-0 flex-1 items-center gap-3"
                >
                  <FlaskConical
                    className="size-3.5 shrink-0 text-muted-foreground"
                    aria-hidden="true"
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[13px] font-medium">
                      {task.kind === "preparation"
                        ? t("knowledge:research.knowledge_check")
                        : task.topic}
                    </span>
                    <span className="block truncate text-xs text-muted-foreground">
                      {task.scope_name === "" ? "—" : task.scope_name} ·{" "}
                      {t("knowledge:research.entries_count", { n: task.entries_created })}
                    </span>
                    {task.check_id === null ? null : (
                      <span className="block truncate text-xs text-muted-foreground">
                        {t("knowledge:research.from_check")}
                      </span>
                    )}
                  </span>
                </Link>
                <StatusBadge status={task.status} variant="canon" />
                {task.status === "ready" ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-7 px-2 text-xs"
                    disabled={defer.isPending}
                    onClick={() =>
                      defer.mutate(task.id, {
                        onError: showError,
                      })
                    }
                  >
                    <CalendarClock className="size-3.5" aria-hidden="true" />
                    {t("knowledge:research.defer")}
                  </Button>
                ) : null}
                {task.status === "deferred" ? (
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-7 px-2 text-xs"
                    disabled={resumeTask.isPending}
                    onClick={() =>
                      resumeTask.mutate(task.id, {
                        onError: showError,
                      })
                    }
                  >
                    <Undo2 className="size-3.5" aria-hidden="true" />
                    {t("knowledge:research.resume")}
                  </Button>
                ) : null}
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
