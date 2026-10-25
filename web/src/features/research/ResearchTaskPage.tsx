/** Research task page (PROJECT §8.10, spec/07 §8, spec/10 §2.5): one task, its state and its run. */
import { useState } from "react"
import { Link, useParams } from "react-router"
import { useTranslation } from "react-i18next"
import { Play, Trash2 } from "lucide-react"

import {
  useDeleteResearchTask,
  usePatchResearchTask,
  useResearchTask,
  useRunResearchTask,
} from "@/api/hooks/research"
import { StatusBadge } from "@/components/common/StatusBadge"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { Textarea } from "@/components/ui/textarea"

export default function ResearchTaskPage() {
  const { t } = useTranslation()
  const { taskId: rawTaskId } = useParams()
  const taskId = Number(rawTaskId)
  const showError = useErrorToast()
  const task = useResearchTask(taskId)
  const patch = usePatchResearchTask()
  const run = useRunResearchTask()
  const remove = useDeleteResearchTask()
  // The author edits a local draft; Save writes it (advancing the draft to ready is separate).
  const [topic, setTopic] = useState<string | null>(null)
  const [requirements, setRequirements] = useState<string | null>(null)

  if (task.isPending || task.data === undefined) {
    return <Skeleton className="m-6 h-64" />
  }

  // A task with an open session is being edited by the researcher (spec/10 §2.5).
  const open = task.data.session_id !== undefined && ["running", "failed"].includes(task.data.status)
  const draftTopic = topic ?? task.data.topic
  const draftRequirements = requirements ?? task.data.requirements
  const dirty =
    draftTopic !== task.data.topic || draftRequirements !== task.data.requirements

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h2 className="truncate text-base font-semibold">{task.data.topic}</h2>
            <StatusBadge status={task.data.status} variant="canon" />
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("knowledge:research.entries_count", { n: task.data.entries_created })}
            {" · "}
            {t("knowledge:research.scope_name")}: {task.data.scope_name}
            {task.data.scope_id === undefined ? ` (${t("knowledge:research.scope_not_resolved")})` : ""}
          </p>
        </div>
        {open && task.data.book_id !== undefined ? (
          // The researcher's stream lives in the book's agent panel, which is where the author
          // watches and controls a session (spec/11 §5.3).
          <Button asChild size="sm" variant="outline">
            <Link to={`/books/${task.data.book_id}/knowledge`}>
              {t("knowledge:research.open_session")}
            </Link>
          </Button>
        ) : (
          <Button
            size="sm"
            onClick={() => run.mutate(taskId, { onError: showError })}
            disabled={run.isPending}
          >
            {run.isPending ? <Spinner /> : <Play className="size-3.5" aria-hidden="true" />}
            {task.data.status === "done" || task.data.status === "cancelled"
              ? t("knowledge:research.rerun")
              : t("knowledge:research.run")}
          </Button>
        )}
      </div>

      {open ? (
        <p className="rounded-md border border-dashed p-2 text-xs text-muted-foreground">
          {t("knowledge:research.blocked_open_session")}
        </p>
      ) : null}

      <div className="space-y-1">
        <Label htmlFor="research-topic">{t("knowledge:research.topic")}</Label>
        <Input
          id="research-topic"
          value={draftTopic}
          disabled={open}
          onChange={(event) => setTopic(event.target.value)}
        />
      </div>
      <div className="space-y-1">
        <Label htmlFor="research-requirements">{t("knowledge:research.requirements")}</Label>
        <Textarea
          id="research-requirements"
          rows={2}
          value={draftRequirements}
          disabled={open}
          onChange={(event) => setRequirements(event.target.value)}
        />
      </div>

      <div className="flex items-center gap-2">
        <Button
          size="sm"
          disabled={!dirty || patch.isPending || open}
          onClick={() => {
            patch.mutate(
              { taskId, patch: { topic: draftTopic, requirements: draftRequirements } },
              {
                onSuccess: () => {
                  setTopic(null)
                  setRequirements(null)
                },
                onError: showError,
              },
            )
          }}
        >
          {patch.isPending ? <Spinner /> : null}
          {t("common:save")}
        </Button>
        {task.data.status === "draft" ? (
          <Button
            size="sm"
            variant="outline"
            disabled={patch.isPending || open}
            onClick={() =>
              patch.mutate(
                { taskId, patch: { status: "ready" } },
                { onError: showError },
              )
            }
          >
            {t("knowledge:research.confirm_ready")}
          </Button>
        ) : null}
        <Button
          size="sm"
          variant="ghost"
          className="ml-auto text-destructive"
          disabled={remove.isPending || open}
          onClick={() => {
            if (!window.confirm(t("knowledge:research.delete_confirm"))) return
            remove.mutate(taskId, {
              onError: showError,
              onSuccess: () => {
                window.location.assign("/research")
              },
            })
          }}
        >
          <Trash2 className="size-3.5" aria-hidden="true" />
          {t("knowledge:research.delete")}
        </Button>
      </div>

      <div className="space-y-1">
        <h3 className="text-sm font-medium">{t("knowledge:research.report")}</h3>
        {task.data.report === "" ? (
          <p className="text-sm text-muted-foreground">{t("knowledge:research.no_report")}</p>
        ) : (
          <p className="whitespace-pre-wrap rounded-md border p-3 text-sm">{task.data.report}</p>
        )}
      </div>
    </div>
  )
}
