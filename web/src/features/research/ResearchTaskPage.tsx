/** Research task page (PROJECT §8.10, spec/07 §8, spec/10 §2.5): one task, its state and its run. */
import { useState } from "react"
import { Link, useParams } from "react-router"
import { useTranslation } from "react-i18next"
import { CalendarClock, Play, Trash2, Undo2 } from "lucide-react"

import {
  useDeferResearchTask,
  useDeleteResearchTask,
  usePatchResearchTask,
  useResearchTask,
  useResumeResearchTask,
  useRunResearchTask,
} from "@/api/hooks/research"
import { useResumeSession, useSession, useSessionControl } from "@/api/hooks/sessions"
import { ActivityStream } from "@/features/agent/ActivityStream"
import { SessionControls } from "@/features/agent/SessionControls"
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
  // The deferred status (D-138): "not now" without running the task or throwing it away.
  const defer = useDeferResearchTask()
  const resumeTask = useResumeResearchTask()
  // The author edits a local draft; Save writes it (advancing the draft to ready is separate).
  const [topic, setTopic] = useState<string | null>(null)
  const [requirements, setRequirements] = useState<string | null>(null)
  // PROJECT §8.10: the research view carries the live agent stream and its controls, so a
  // book-less task (which has no agent panel) can still be watched and stopped. The control hooks
  // only invalidate session query keys; a book-less task passes 0, which matches no cached key.
  const sessionId =
    task.data?.session_id !== undefined &&
    ["running", "failed"].includes(task.data?.status ?? "")
      ? (task.data.session_id ?? null)
      : null
  const snapshot = useSession(sessionId)
  const control = useSessionControl(task.data?.book_id ?? 0)
  const resume = useResumeSession(task.data?.book_id ?? 0)

  if (task.isPending || task.data === undefined) {
    return <Skeleton className="m-6 h-64" />
  }

  // Preparation tasks are the knowledge check: their "topic" is a fixed title and neither their
  // topic nor their scope is editable from this page (spec/11 §5.5, D-158).
  const isPreparation = task.data.kind === "preparation"

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
            <h2 className="truncate text-base font-semibold">
              {isPreparation ? t("knowledge:research.knowledge_check") : task.data.topic}
            </h2>
            <StatusBadge status={task.data.status} variant="canon" />
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("knowledge:research.entries_count", { n: task.data.entries_created })}
            {" · "}
            {t("knowledge:research.scope_name")}:{" "}
            {task.data.scope_name === "" ? "—" : task.data.scope_name}
            {!isPreparation && task.data.scope_id === undefined
              ? ` (${t("knowledge:research.scope_not_resolved")})`
              : ""}
          </p>
        </div>
        {open ? (
          task.data.book_id !== undefined ? (
            // The same session also lives in the book's agent panel, which has the full stream.
            <Button asChild size="sm" variant="outline">
              <Link to={`/books/${task.data.book_id}/knowledge`}>
                {t("knowledge:research.open_session")}
              </Link>
            </Button>
          ) : null
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

      {open && sessionId !== null && snapshot.data !== undefined ? (
        <div className="flex h-[24rem] flex-col overflow-hidden rounded-md border">
          <SessionControls
            session={snapshot.data.session}
            busy={control.isPending || resume.isPending}
            onPause={() =>
              control.mutate({ sessionId, operation: "pause" }, { onError: showError })
            }
            onStop={() =>
              control.mutate({ sessionId, operation: "stop" }, { onError: showError })
            }
            onContinue={() =>
              control.mutate({ sessionId, operation: "continue" }, { onError: showError })
            }
            onFeedback={(feedback) =>
              resume.mutate({ sessionId, feedback }, { onError: showError })
            }
            onAbandon={() => {
              if (!window.confirm(t("knowledge:research.abandon_confirm"))) return
              control.mutate({ sessionId, operation: "abandon" }, { onError: showError })
            }}
          />
          <ActivityStream
            session={snapshot.data.session}
            messages={snapshot.data.messages}
            inbox={snapshot.data.inbox}
            nextCursor={snapshot.data.next_cursor}
          />
        </div>
      ) : null}

      {open ? (
        <p className="rounded-md border border-dashed p-2 text-xs text-muted-foreground">
          {t("knowledge:research.blocked_open_session")}
        </p>
      ) : null}

      {task.data.status === "deferred" && !open ? (
        <p className="rounded-md border border-dashed p-2 text-xs text-muted-foreground">
          {t("knowledge:research.deferred_hint")}
        </p>
      ) : null}

      {isPreparation ? null : (
        <>
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
        </>
      )}

      <div className="flex items-center gap-2">
        {isPreparation ? null : (
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
        )}
        {task.data.status === "ready" ? (
          <Button
            size="sm"
            variant="outline"
            disabled={defer.isPending || open}
            onClick={() =>
              defer.mutate(taskId, {
                onError: showError,
              })
            }
          >
            {defer.isPending ? (
              <Spinner />
            ) : (
              <CalendarClock className="size-3.5" aria-hidden="true" />
            )}
            {t("knowledge:research.defer")}
          </Button>
        ) : null}
        {task.data.status === "deferred" ? (
          <Button
            size="sm"
            variant="outline"
            disabled={resumeTask.isPending || open}
            onClick={() =>
              resumeTask.mutate(taskId, {
                onError: showError,
              })
            }
          >
            {resumeTask.isPending ? (
              <Spinner />
            ) : (
              <Undo2 className="size-3.5" aria-hidden="true" />
            )}
            {t("knowledge:research.resume")}
          </Button>
        ) : null}
        {!isPreparation && task.data.status === "draft" ? (
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

      {task.data.status === "done" && task.data.entries.length > 0 ? (
        <div className="space-y-1">
          <h3 className="text-sm font-medium">{t("knowledge:research.created_entries")}</h3>
          <ul className="space-y-1 rounded-md border p-3 text-sm">
            {task.data.entries.map((entry) => (
              <li key={entry.id} className="truncate">
                <Link
                  // Entries are shown inside the knowledge pages, which open the scope's entry
                  // list; the research task has no per-entry route of its own.
                  to={
                    task.data.book_id !== undefined
                      ? `/books/${task.data.book_id}/knowledge`
                      : "/knowledge"
                  }
                  className="text-primary underline-offset-2 hover:underline"
                >
                  {entry.title}
                </Link>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  )
}
