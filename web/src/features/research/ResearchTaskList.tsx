/** The research task list (PROJECT §8.10): the tasks of one book, or every task. */
import { Link } from "react-router"
import { useTranslation } from "react-i18next"
import { FlaskConical, Plus } from "lucide-react"

import { useResearchTasks } from "@/api/hooks/research"
import { EmptyState } from "@/components/common/EmptyState"
import { StatusBadge } from "@/components/common/StatusBadge"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"

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

  if (tasks.isPending) {
    return <Skeleton className="h-24" />
  }
  const items = tasks.data ?? []

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-medium">{t("knowledge:research.list_title")}</h3>
        {onCreate === undefined ? null : (
          <Button size="sm" variant="outline" onClick={onCreate}>
            <Plus className="size-3.5" aria-hidden="true" />
            {t("knowledge:research.new_task")}
          </Button>
        )}
      </div>
      {items.length === 0 ? (
        <EmptyState title={t("knowledge:research.empty")} description={t("knowledge:research.empty_hint")} />
      ) : (
        <ul className="divide-y rounded-md border">
          {items.map((task) => (
            <li key={task.id}>
              <Link
                to={`/research/${task.id}`}
                className="flex items-center gap-3 px-3 py-2 hover:bg-accent"
              >
                <FlaskConical className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[13px] font-medium">{task.topic}</span>
                  <span className="block truncate text-xs text-muted-foreground">
                    {task.scope_name} · {t("knowledge:research.entries_count", { n: task.entries_created })}
                  </span>
                </span>
                <StatusBadge status={task.status} variant="canon" />
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
