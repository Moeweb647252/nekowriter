/**
 * Research before conception (spec/11 §5.4, D-168).
 *
 * The knowledge check plans topics; the topics then run one at a time, and conception starts only
 * when they settle. This component owns that pipeline: the six states of plan §3.8, the topic list
 * and its editing, and the check/topic streams with their session controls.
 */
import { useMemo, useState } from "react"
import { Link } from "react-router"
import { useTranslation } from "react-i18next"
import ReactMarkdown from "react-markdown"
import remarkGfm from "remark-gfm"
import { ExternalLink, Loader2, Plus } from "lucide-react"

import type { BookDto } from "@/api/gen/BookDto"
import type { InboxItemDto } from "@/api/gen/InboxItemDto"
import type { MessageWithCallsDto } from "@/api/gen/MessageWithCallsDto"
import type { ResearchTaskDto } from "@/api/gen/ResearchTaskDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import {
  useCreateResearchTask,
  useDeferResearchTask,
  useDeleteResearchTask,
  usePatchResearchTask,
  useResumeResearchTask,
  useRunResearchTask,
} from "@/api/hooks/research"
import {
  useResumeSession,
  useSession,
  useSessionControl,
} from "@/api/hooks/sessions"
import { ActivityStream } from "@/features/agent/ActivityStream"
import { SessionControls } from "@/features/agent/SessionControls"
import { SessionErrorNotice } from "@/features/agent/SessionErrorNotice"
import { StatusBadge } from "@/components/common/StatusBadge"
import { SessionStatusBadge } from "@/features/agent/SessionStatusBadge"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"

export type PreConceptionResearchProps = {
  bookId: number
  book: BookDto
  check: ResearchTaskDto | undefined
  checkSession: SessionDto | undefined
  checkSnapshot:
    | { messages: MessageWithCallsDto[]; inbox: InboxItemDto[]; next_cursor?: string | null }
    | undefined
  topics: ResearchTaskDto[]
  startPending: boolean
  checkPending: boolean
  continuePending: boolean
  controlsBusy: boolean
  onPause: (session: SessionDto) => void
  onStop: (session: SessionDto) => void
  onContinue: (session: SessionDto) => void
  onFeedback: (session: SessionDto, feedback: string) => void
  onAbandon: (session: SessionDto) => void
  onStart: () => void
  onCheck: () => void
  onContinueCheck: (taskId: number) => void
}

/** The check's topics: the book's tasks that carry its `check_id`, in id order. */
export function checkTopics(topics: ResearchTaskDto[], checkId: number): ResearchTaskDto[] {
  // `GET /research-tasks` is newest-first; the queue and spec/11 §5.4 use id order.
  return topics
    .filter((topic) => topic.check_id === checkId)
    .sort((left, right) => left.id - right.id)
}

/** True while a topic still waits for the queue (`ready`). */
export function isWaiting(topic: ResearchTaskDto): boolean {
  return topic.status === "ready"
}

/** The open topic: running, or failed with a session whose controls can continue it. */
export function openTopic(topics: ResearchTaskDto[]): ResearchTaskDto | undefined {
  return topics.find(
    (topic) =>
      topic.status === "running" || (topic.status === "failed" && topic.session_id != null),
  )
}

function Progress({ done, total }: { done: number; total: number }) {
  const { t } = useTranslation()
  return (
    <span className="text-xs text-muted-foreground">
      {t("book:conception.research_before.progress", { done, total })}
    </span>
  )
}

/** The pane before a conception session exists (spec/11 §5.4). */
export function PreConceptionResearch({
  bookId,
  book,
  check,
  checkSession,
  checkSnapshot,
  topics,
  startPending,
  checkPending,
  continuePending,
  controlsBusy,
  onPause,
  onStop,
  onContinue,
  onFeedback,
  onAbandon,
  onStart,
  onCheck,
  onContinueCheck,
}: PreConceptionResearchProps) {
  const { t } = useTranslation()
  const checkTopicsList = useMemo(
    () => (check === undefined ? [] : checkTopics(topics, check.id)),
    [topics, check],
  )
  const runningTopic = openTopic(checkTopicsList)
  const waiting = checkTopicsList.find(isWaiting)
  const done = checkTopicsList.filter((topic) => topic.status === "done").length
  const settled = runningTopic === undefined && waiting === undefined
  const topicSessionId = runningTopic?.session_id ?? null
  const topicSnapshot = useSession(topicSessionId)
  const topicControl = useSessionControl(bookId)
  const topicResume = useResumeSession(bookId)
  const topicRun = useRunResearchTask(bookId)
  const showError = useErrorToast()

  const startButton = (primary: boolean) => (
    <Button
      size="sm"
      variant={primary ? "default" : "outline"}
      disabled={startPending}
      onClick={onStart}
    >
      {startPending ? t("common:saving") : t("book:conception.start")}
    </Button>
  )

  if (check === undefined) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center text-sm text-muted-foreground">
        <p>{t("book:conception.no_session")}</p>
        {book.status === "conceiving" ? (
          <div className="flex flex-wrap items-center justify-center gap-2">
            {startButton(book.conception.start_after_knowledge_check !== true)}
            <Button size="sm" variant="outline" disabled={checkPending} onClick={onCheck}>
              {checkPending
                ? t("common:saving")
                : t("book:conception.knowledge_check.check_first")}
            </Button>
          </div>
        ) : null}
      </div>
    )
  }

  const topicList = (
    <TopicList bookId={bookId} checkId={check.id} topics={checkTopicsList} />
  )

  if (check.status === "failed" || check.status === "cancelled") {
    return (
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-4" data-testid={`knowledge-check-${check.status}`}>
        <ResearchHeader check={check} session={checkSession} />
        <SessionErrorNotice error={checkSession?.error ?? null} />
        <p className="text-sm text-muted-foreground">
          {t(`agent:status.${check.status}`, { defaultValue: check.status })}
        </p>
        <div className="flex flex-wrap items-center gap-2">
          {check.status === "failed" && check.session_id != null ? (
            <Button
              size="sm"
              variant="outline"
              disabled={continuePending}
              onClick={() => onContinueCheck(check.id)}
            >
              {continuePending
                ? t("common:saving")
                : t("book:conception.knowledge_check.continue")}
            </Button>
          ) : null}
          {startButton(book.conception.start_after_knowledge_check !== true)}
        </div>
        {checkTopicsList.some(isWaiting) ? (
          <p className="text-xs text-muted-foreground">
            {t("book:conception.research_before.run_on_task_page")}
          </p>
        ) : null}
        {topicList}
      </div>
    )
  }

  // C is still open (planning): the check stream plus the list filling in live.
  if (check.status !== "done") {
    return (
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-4" data-testid="knowledge-check-open">
        <ResearchHeader check={check} session={checkSession} />
        <SessionErrorNotice error={checkSession?.error ?? null} />
        {checkSession === undefined ? (
          <div className="flex flex-1 items-center justify-center">
            <Loader2 className="size-4 animate-spin text-muted-foreground" aria-hidden="true" />
          </div>
        ) : (
          <CheckStream
            session={checkSession}
            snapshot={checkSnapshot}
            busy={controlsBusy}
            onPause={() => onPause(checkSession)}
            onStop={() => onStop(checkSession)}
            onContinue={() => onContinue(checkSession)}
            onFeedback={(feedback) => onFeedback(checkSession, feedback)}
            onAbandon={() => onAbandon(checkSession)}
          />
        )}
        {topicList}
        <div className="flex justify-end">
          <Button
            size="sm"
            disabled={startPending}
            onClick={() => {
              if (!window.confirm(t("book:conception.knowledge_check.still_running_confirm"))) {
                return
              }
              onStart()
            }}
          >
            {startPending
              ? t("common:saving")
              : t("book:conception.knowledge_check.start_now")}
          </Button>
        </div>
      </div>
    )
  }

  // C is done and a topic is open or waiting: the queue is running.
  if (!settled) {
    return (
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-4" data-testid="research-before-conception">
        <div className="flex flex-wrap items-center gap-2">
          <h2 className="text-sm font-semibold">
            {t("book:conception.research_before.title")}
          </h2>
          <Progress done={done} total={checkTopicsList.length} />
        </div>
        <CheckReport report={check.report} />
        {runningTopic === undefined ? null : (
          <>
            <SessionErrorNotice error={topicSnapshot.data?.session.error ?? null} />
            {topicSnapshot.data === undefined ? (
              <div className="flex flex-1 items-center justify-center">
                <Loader2 className="size-4 animate-spin text-muted-foreground" aria-hidden="true" />
              </div>
            ) : (
              <div className="flex h-[24rem] shrink-0 flex-col overflow-hidden rounded-md border">
                {runningTopic.status === "failed" ? null : (
                  <SessionControls
                    session={topicSnapshot.data.session}
                    busy={topicControl.isPending || topicResume.isPending}
                    onPause={() =>
                      topicControl.mutate(
                        { sessionId: runningTopic.session_id ?? 0, operation: "pause" },
                        { onError: showError },
                      )
                    }
                    onStop={() =>
                      topicControl.mutate(
                        { sessionId: runningTopic.session_id ?? 0, operation: "stop" },
                        { onError: showError },
                      )
                    }
                    onContinue={() => {
                      const current = topicSnapshot.data?.session
                      if (current === undefined) return
                      if (current.status === "held" || current.status === "paused") {
                        topicResume.mutate(
                          { sessionId: current.id },
                          { onError: showError },
                        )
                      } else {
                        topicControl.mutate(
                          { sessionId: current.id, operation: "continue" },
                          { onError: showError },
                        )
                      }
                    }}
                    onFeedback={(feedback) =>
                      topicResume.mutate(
                        { sessionId: runningTopic.session_id ?? 0, feedback },
                        { onError: showError },
                      )
                    }
                    onAbandon={() =>
                      topicControl.mutate(
                        { sessionId: runningTopic.session_id ?? 0, operation: "abandon" },
                        { onError: showError },
                      )
                    }
                  />
                )}
                <ActivityStream
                  session={topicSnapshot.data.session}
                  messages={topicSnapshot.data.messages}
                  inbox={topicSnapshot.data.inbox}
                  nextCursor={topicSnapshot.data.next_cursor}
                  paneName="research-before-conception"
                />
              </div>
            )}
            {runningTopic.status === "failed" ? (
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={continuePending}
                  onClick={() => topicRun.mutate(runningTopic.id, { onError: showError })}
                >
                  {t("book:conception.research_before.continue")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    if (!window.confirm(t("knowledge:research.abandon_confirm"))) return
                    topicControl.mutate(
                      { sessionId: runningTopic.session_id ?? 0, operation: "abandon" },
                      { onError: showError },
                    )
                  }}
                >
                  {t("book:conception.research_before.abandon")}
                </Button>
              </div>
            ) : null}
          </>
        )}
        {runningTopic === undefined && waiting !== undefined ? (
          <div className="flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              variant="outline"
              onClick={() => topicRun.mutate(waiting.id, { onError: showError })}
            >
              {t("book:conception.research_before.run_next")}
            </Button>
            <span className="text-xs text-muted-foreground">
              {t("book:conception.research_before.stall_hint")}
            </span>
          </div>
        ) : null}
        {topicList}
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="text-xs text-muted-foreground">
            {book.conception.start_after_knowledge_check
              ? t("book:conception.research_before.hint_auto")
              : t("book:conception.research_before.hint_manual")}
          </p>
          <Button
            size="sm"
            variant="outline"
            disabled={startPending}
            onClick={() => {
              if (!window.confirm(t("book:conception.knowledge_check.still_running_confirm"))) {
                return
              }
              onStart()
            }}
          >
            {startPending
              ? t("common:saving")
              : t("book:conception.knowledge_check.start_now")}
          </Button>
        </div>
      </div>
    )
  }

  // C is done and T settled.
  const entries = checkTopicsList.reduce((total, topic) => total + topic.entries_created, 0)
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-4" data-testid="research-conception-settled">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-sm font-semibold">{t("book:conception.research_before.title")}</h2>
      </div>
      <CheckReport report={check.report} />
      {topicList}
      <p className="text-xs text-muted-foreground">
        {t("book:conception.research_before.entries_total", { count: entries })}
      </p>
      <Link
        to={`/books/${bookId}/knowledge`}
        className="text-xs text-primary underline-offset-2 hover:underline"
      >
        {t("book:conception.knowledge_check.open_knowledge")}
      </Link>
      <div className="mt-auto flex justify-end">
        {startButton(book.conception.start_after_knowledge_check !== true)}
      </div>
    </div>
  )
}

function ResearchHeader({
  check,
  session,
}: {
  check: ResearchTaskDto
  session: SessionDto | undefined
}) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-wrap items-center gap-2">
      <h2 className="text-sm font-semibold">
        {check.status === "done"
          ? t("book:conception.research_before.title")
          : t("book:conception.knowledge_check.title")}
      </h2>
      {session === undefined ? (
        <StatusBadge status={check.status} variant="canon" />
      ) : (
        <SessionStatusBadge status={session.status} />
      )}
      <Link
        to={`/research/${check.id}`}
        className="inline-flex items-center gap-1 text-xs text-primary underline-offset-2 hover:underline"
      >
        <ExternalLink className="size-3.5" aria-hidden="true" />
        {t("book:conception.knowledge_check.open_task")}
      </Link>
    </div>
  )
}

function CheckReport({ report }: { report: string }) {
  const { t } = useTranslation()
  return (
    <details className="rounded-md border p-3">
      <summary className="cursor-pointer text-sm font-medium">
        {t("book:conception.research_before.report")}
      </summary>
      {report === "" ? (
        <p className="mt-2 text-sm text-muted-foreground">
          {t("book:conception.knowledge_check.no_report")}
        </p>
      ) : (
        <div className="prose-cjk mt-2 text-sm">
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{report}</ReactMarkdown>
        </div>
      )}
    </details>
  )
}

function CheckStream({
  session,
  snapshot,
  busy,
  onPause,
  onStop,
  onContinue,
  onFeedback,
  onAbandon,
}: {
  session: SessionDto
  snapshot: PreConceptionResearchProps["checkSnapshot"]
  busy: boolean
  onPause: () => void
  onStop: () => void
  onContinue: () => void
  onFeedback: (feedback: string) => void
  onAbandon: () => void
}) {
  return (
    <div className="flex h-[24rem] shrink-0 flex-col overflow-hidden rounded-md border">
      <SessionControls
        session={session}
        busy={busy}
        onPause={onPause}
        onStop={onStop}
        onContinue={onContinue}
        onFeedback={onFeedback}
        onAbandon={onAbandon}
      />
      <ActivityStream
        session={session}
        messages={snapshot?.messages ?? []}
        inbox={snapshot?.inbox ?? []}
        nextCursor={snapshot?.next_cursor}
        paneName="knowledge-check"
      />
    </div>
  )
}

/** The topic list with inline editing and the Add-topic form. */
function TopicList({
  bookId,
  checkId,
  topics,
}: {
  bookId: number
  checkId: number
  topics: ResearchTaskDto[]
}) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const patch = usePatchResearchTask()
  const remove = useDeleteResearchTask()
  const defer = useDeferResearchTask()
  const resume = useResumeResearchTask()
  const create = useCreateResearchTask()
  const [editing, setEditing] = useState<number | null>(null)
  const [topic, setTopic] = useState("")
  const [requirements, setRequirements] = useState("")
  const [scope, setScope] = useState("")
  const [adding, setAdding] = useState(false)
  const [newTopic, setNewTopic] = useState("")
  const [newRequirements, setNewRequirements] = useState("")
  const [newScope, setNewScope] = useState("")

  const startEdit = (item: ResearchTaskDto) => {
    setEditing(item.id)
    setTopic(item.topic)
    setRequirements(item.requirements)
    setScope(item.scope_name)
  }

  const submitEdit = (item: ResearchTaskDto) => {
    patch.mutate(
      {
        taskId: item.id,
        patch: { topic, requirements, scope_name: scope },
      },
      { onSuccess: () => setEditing(null), onError: showError },
    )
  }

  return (
    <section className="space-y-2" data-testid="check-topic-list">
      <h3 className="text-sm font-medium">
        {t("book:conception.research_before.topics")}
      </h3>
      {topics.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          {t("book:conception.research_before.no_topics")}
        </p>
      ) : (
        <ul className="space-y-2">
          {topics.map((item) => (
            <li
              key={item.id}
              data-testid={`check-topic-${item.id}`}
              className="rounded-md border p-2 text-sm"
            >
              <div className="flex flex-wrap items-center gap-2">
                <StatusBadge status={item.status} variant="canon" />
                <Link
                  to={`/research/${item.id}`}
                  className="min-w-0 flex-1 truncate font-medium underline-offset-2 hover:underline"
                >
                  {item.topic}
                </Link>
                <span className="text-xs text-muted-foreground">
                  {item.scope_name === "" ? "—" : item.scope_name}
                </span>
                {item.origin === "author" ? (
                  <span className="text-xs text-muted-foreground">
                    {t("book:conception.added_by_you")}
                  </span>
                ) : null}
              </div>
              {item.requirements === "" ? null : (
                <details className="mt-1 text-xs text-muted-foreground">
                  <summary className="cursor-pointer">
                    {t("book:conception.topic_requirements")}
                  </summary>
                  <p className="mt-1 whitespace-pre-wrap">{item.requirements}</p>
                </details>
              )}
              {editing === item.id && item.status === "ready" ? (
                <div className="mt-2 space-y-2">
                  <Input
                    aria-label={t("book:conception.topic")}
                    value={topic}
                    onChange={(event) => setTopic(event.target.value)}
                  />
                  <Textarea
                    aria-label={t("book:conception.topic_requirements")}
                    rows={2}
                    value={requirements}
                    onChange={(event) => setRequirements(event.target.value)}
                  />
                  <Input
                    aria-label={t("book:conception.topic_scope")}
                    value={scope}
                    onChange={(event) => setScope(event.target.value)}
                  />
                  <div className="flex justify-end gap-1">
                    <Button size="sm" variant="ghost" onClick={() => setEditing(null)}>
                      {t("common:cancel")}
                    </Button>
                    <Button size="sm" onClick={() => submitEdit(item)}>
                      {t("common:save")}
                    </Button>
                  </div>
                </div>
              ) : null}
              <div className="mt-2 flex flex-wrap items-center gap-1">
                {item.status === "ready" ? (
                  <>
                    <Button size="sm" variant="outline" onClick={() => startEdit(item)}>
                      {t("common:edit")}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => defer.mutate(item.id, { onError: showError })}
                    >
                      {t("book:conception.research_before.skip")}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="text-destructive"
                      onClick={() => {
                        if (!window.confirm(t("book:conception.research_before.delete_confirm"))) {
                          return
                        }
                        remove.mutate(item.id, { onError: showError })
                      }}
                    >
                      {t("common:delete")}
                    </Button>
                  </>
                ) : null}
                {item.status === "deferred" ? (
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => resume.mutate(item.id, { onError: showError })}
                  >
                    {t("book:conception.research_before.resume")}
                  </Button>
                ) : null}
                {item.status === "done" ? (
                  <>
                    <span className="text-xs text-muted-foreground">
                      {t("knowledge:research.entries_count", { n: item.entries_created })}
                    </span>
                    <Link
                      to={`/research/${item.id}`}
                      className="text-xs text-primary underline-offset-2 hover:underline"
                    >
                      {t("book:conception.research_before.open_task")}
                    </Link>
                  </>
                ) : null}
              </div>
            </li>
          ))}
        </ul>
      )}
      {adding ? (
        <div className="space-y-2 rounded-md border p-2">
          <Input
            aria-label={t("book:conception.topic")}
            value={newTopic}
            onChange={(event) => setNewTopic(event.target.value)}
          />
          <Textarea
            aria-label={t("book:conception.topic_requirements")}
            rows={2}
            value={newRequirements}
            onChange={(event) => setNewRequirements(event.target.value)}
          />
          <Input
            aria-label={t("book:conception.topic_scope")}
            value={newScope}
            onChange={(event) => setNewScope(event.target.value)}
          />
          <div className="flex justify-end gap-1">
            <Button size="sm" variant="ghost" onClick={() => setAdding(false)}>
              {t("common:cancel")}
            </Button>
            <Button
              size="sm"
              disabled={newTopic.trim() === "" || newScope.trim() === ""}
              onClick={() =>
                create.mutate(
                  {
                    book_id: bookId,
                    check_id: checkId,
                    scope_name: newScope.trim(),
                    topic: newTopic.trim(),
                    requirements: newRequirements,
                  },
                  {
                    onSuccess: () => {
                      setAdding(false)
                      setNewTopic("")
                      setNewRequirements("")
                      setNewScope("")
                    },
                    onError: showError,
                  },
                )
              }
            >
              {t("book:conception.add_topic")}
            </Button>
          </div>
        </div>
      ) : (
        <Button size="sm" variant="outline" onClick={() => setAdding(true)}>
          <Plus className="size-3.5" aria-hidden="true" />
          {t("book:conception.add_topic")}
        </Button>
      )}
    </section>
  )
}
