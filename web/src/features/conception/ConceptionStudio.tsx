/**
 * The conception studio (PROJECT §8.7, spec/11 §5.4): conversation on the left, the live setting
 * materials on the right.
 *
 * - The conversation reuses the agent panel's `ActivityStream`, `Composer` and tool cards, so the
 *   tool cards, live deltas and appendices are the same components as everywhere else.
 * - A waiting form (`ask_user` / `request_review`) is answered with `POST /sessions/:id/answer`;
 *   it is the reply at that moment (PROJECT §8.7 step 5), so the composer is disabled then.
 * - The documents are the three setting docs, the character designs and the draft research topics.
 *   They are editable whenever the conception session is not running (spec/08 §1.4); a direct
 *   edit is reported to the agent at its next turn (spec/03 §7).
 * - "Finalize book" (zh-CN 定稿, D-060) is enabled once all four documents are non-empty; when an
 *   item was never approved, the author confirms first (PROJECT §8.7 step 6).
 */
import { useMemo, useState } from "react"
import { Link, useNavigate, useParams } from "react-router"
import { useTranslation } from "react-i18next"
import ReactMarkdown from "react-markdown"
import remarkGfm from "remark-gfm"
import { ExternalLink, Loader2 } from "lucide-react"

import {
  useAnswerSession,
  useBookSessions,
  useResumeSession,
  useSendSessionMessage,
  useSession,
  useSessionControl,
} from "@/api/hooks/sessions"
import { useCharacters, useDeleteCharacter, usePatchCharacter, useReorderCharacters, useSettingDocs, usePutSettingDoc } from "@/api/hooks/settings-materials"
import {
  useCreateResearchTask,
  useDeleteResearchTask,
  usePatchResearchTask,
  useResearchTasks,
  useStartKnowledgeCheck,
} from "@/api/hooks/research"
import { latestPreparationTask, knowledgeCheckOpen } from "@/features/research/researchTask"
export { latestPreparationTask, knowledgeCheckOpen }
import { useBook } from "@/api/hooks/books"
import { useFinalizeConception, useStartConception } from "@/api/hooks/conception"
import { useErrorToast, useWarningToasts } from "@/components/common/ErrorToast"
import { ConfirmDialog } from "@/components/common/ConfirmDialog"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { ActivityStream } from "@/features/agent/ActivityStream"
import { Composer } from "@/features/agent/Composer"
import { SessionControls } from "@/features/agent/SessionControls"
import { SessionErrorNotice } from "@/features/agent/SessionErrorNotice"
import { AskUserForm, readQuestions } from "@/features/conception/AskUserForm"
import { CharacterRankList } from "@/features/conception/CharacterRankList"
import { ResearchTopicsEditor } from "@/features/conception/ResearchTopicsEditor"
import {
  REVIEW_ITEMS,
  ReviewRequestForm,
  type ReviewItem,
} from "@/features/conception/ReviewRequestForm"
import type { MessageWithCallsDto } from "@/api/gen/MessageWithCallsDto"
import type { SessionAnswerInput } from "@/api/gen/SessionAnswerInput"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import type { SettingDocKind } from "@/api/gen/SettingDocKind"
import type { ResearchTaskDto } from "@/api/gen/ResearchTaskDto"
import type { BookDto } from "@/api/gen/BookDto"
import { SessionStatusBadge } from "@/features/agent/SessionStatusBadge"
import { StatusBadge } from "@/components/common/StatusBadge"

/** The session statuses that allow the author to edit the materials (spec/08 §1.4). */
const EDITABLE_STATUSES = new Set([
  "idle",
  "held",
  "awaiting_input",
  "paused",
  "stopped",
  "failed",
  "interrupted",
])

/** The awaiting call of a session snapshot, when there is one. */
export function awaitingCall(session: SessionDto, messages: MessageWithCallsDto[]): ToolCallDto | null {
  if (session.status !== "awaiting_input") {
    return null
  }
  for (const item of messages) {
    for (const call of item.tool_calls) {
      if (call.status === "awaiting_input") {
        return call
      }
    }
  }
  return null
}

/** True when every required material is non-empty (PROJECT §8.7 step 6). */
export function materialsComplete(input: {
  docs: { kind: string; content: string }[]
  characters: { id: number }[]
}): boolean {
  const filled = (kind: string) =>
    input.docs.find((doc) => doc.kind === kind)?.content.trim() !== "" &&
    input.docs.some((doc) => doc.kind === kind)
  return filled("outline") && filled("world") && filled("protagonist") && input.characters.length > 0
}

export default function ConceptionStudio() {
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const { t } = useTranslation()
  const navigate = useNavigate()
  const showError = useErrorToast()
  const showWarnings = useWarningToasts()

  const book = useBook(bookId)
  const sessions = useBookSessions(bookId)
  const docs = useSettingDocs(bookId)
  const characters = useCharacters(bookId)
  const topics = useResearchTasks({ bookId })
  const latestCheck = useMemo(
    () => latestPreparationTask(topics.data ?? []),
    [topics.data],
  )
  const checkSessionId = latestCheck?.session_id ?? null
  const checkSnapshot = useSession(checkSessionId)

  // The book's open conception session: the newest one that is not finished (spec/02 §5).
  const conception = useMemo<SessionDto | undefined>(
    () =>
      (sessions.data ?? []).find(
        (session) => session.kind === "conception" && session.status !== "abandoned" && session.status !== "superseded",
      ),
    [sessions.data],
  )
  const sessionId = conception?.id ?? null
  const snapshot = useSession(sessionId)
  const session = snapshot.data?.session ?? conception
  const messages = snapshot.data?.messages ?? []

  const answer = useAnswerSession(bookId)
  const send = useSendSessionMessage(bookId)
  // The studio is where the conception session is used, so it needs the same Pause/Stop/
  // Continue/Feedback/Abandon controls as the agent panel (PROJECT §8.6, §8.7).
  const control = useSessionControl(bookId)
  const resume = useResumeSession(bookId)
  const finalize = useFinalizeConception(bookId)
  const start = useStartConception(bookId)
  const startKnowledgeCheck = useStartKnowledgeCheck(bookId)
  // One document hook per kind (the endpoint is per kind); `saveDoc` picks the right one.
  const putOutline = usePutSettingDoc(bookId, "outline")
  const putWorld = usePutSettingDoc(bookId, "world")
  const putProtagonist = usePutSettingDoc(bookId, "protagonist")
  const reorder = useReorderCharacters(bookId)
  const patchCharacter = usePatchCharacter(bookId)
  const deleteCharacter = useDeleteCharacter(bookId)
  const createTopic = useCreateResearchTask()
  const patchTopic = usePatchResearchTask()
  const deleteTopic = useDeleteResearchTask()

  const [confirmFinalize, setConfirmFinalize] = useState(false)
  const [activeTab, setActiveTab] = useState<string>("outline")
  const [docDraft, setDocDraft] = useState<{ kind: string; revision: number; content: string } | null>(
    null,
  )

  if (book.isPending || sessions.isPending || docs.isPending || characters.isPending || topics.isPending) {
    return <Skeleton className="m-6 h-64" />
  }
  if (book.data === undefined) {
    return <p className="p-6 text-sm text-muted-foreground">{t("common:error_body")}</p>
  }

  const running = session?.status === "running"
  const editable = !running && (session === undefined || EDITABLE_STATUSES.has(session.status))
  const docList = docs.data ?? []
  const characterList = characters.data ?? []
  const topicList = (topics.data ?? []).filter((topic) => topic.status === "draft")
  const complete = materialsComplete({ docs: docList, characters: characterList })
  const checkSession =
    checkSnapshot.data?.session ??
    (sessions.data ?? []).find((candidate) => candidate.id === checkSessionId)

  /** The review items the book actually has (the three docs are always present). */
  const reviewItems: ReviewItem[] = REVIEW_ITEMS.filter((item) =>
    item === "characters" ? characterList.length > 0 : item === "research_topics" ? topicList.length > 0 : true,
  )

  const anyUnapproved =
    docList.some((doc) => doc.approved_revision !== doc.revision) ||
    (characterList.length > 0 && book.data.conception.characters_approved_revision !== book.data.conception.characters_revision) ||
    (topicList.length > 0 && book.data.conception.topics_approved_revision !== book.data.conception.topics_revision)

  const doFinalize = () =>
    finalize.mutate(undefined, {
      onSuccess: (response) => {
        showWarnings(response.warnings ?? [])
        setConfirmFinalize(false)
        void navigate(`/books/${response.book.id}`)
      },
      onError: showError,
    })

  const awaitCall =
    session === undefined ? null : awaitingCall(session, messages)

  const submitAnswer = (payload: SessionAnswerInput["payload"]) => {
    if (session === undefined || awaitCall === null) {
      return
    }
    answer.mutate(
      { sessionId: session.id, input: { tool_call_id: awaitCall.id, payload } },
      { onError: showError },
    )
  }
  const busy =
    answer.isPending ||
    send.isPending ||
    control.isPending ||
    resume.isPending ||
    finalize.isPending ||
    reorder.isPending ||
    patchCharacter.isPending ||
    deleteCharacter.isPending ||
    createTopic.isPending ||
    patchTopic.isPending ||
    deleteTopic.isPending

  return (
    /*
      The studio is a bounded two-pane workspace, not a page (PROJECT §8.7, spec/11 §5.4): the
      header, the session controls, any pending form, the composer and the finalize bar stay put
      while the conversation history and the document editor scroll on their own. `h-full` needs a
      definite ancestor height, which `BookLayout` gives the studio route only; `overflow-hidden`
      makes sure nothing can grow past the pane and push the composer out of view.
    */
    <div data-testid="conception-studio" className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="flex min-h-0 flex-1 flex-col gap-0 lg:flex-row">
        {/* Conversation */}
        <section className="flex min-h-0 min-w-0 flex-1 flex-col border-r">
          <header className="flex items-center justify-between gap-2 border-b px-3 py-2">
            <div className="min-w-0">
              <h1 className="truncate text-sm font-semibold">{t("book:conception.title")}</h1>
              <p className="truncate text-xs text-muted-foreground">
                {t("book:conception.subtitle")}
              </p>
            </div>
            {running ? (
              <span className="flex items-center gap-1 text-xs text-muted-foreground">
                <Loader2 className="size-3 animate-spin" />
                {t("book:conception.working")}
              </span>
            ) : null}
          </header>
          {session === undefined ? null : (
            <SessionControls
              session={session}
              busy={busy}
              onPause={() =>
                control.mutate({ sessionId: session.id, operation: "pause" }, { onError: showError })
              }
              onStop={() =>
                control.mutate({ sessionId: session.id, operation: "stop" }, { onError: showError })
              }
              onContinue={() => {
                // Held/paused sessions resume; stopped/failed/interrupted ones continue
                // (spec/03 §3).
                if (session.status === "held" || session.status === "paused") {
                  resume.mutate({ sessionId: session.id }, { onError: showError })
                } else {
                  control.mutate(
                    { sessionId: session.id, operation: "continue" },
                    { onError: showError },
                  )
                }
              }}
              onFeedback={(feedback) =>
                resume.mutate({ sessionId: session.id, feedback }, { onError: showError })
              }
              onAbandon={() =>
                control.mutate(
                  { sessionId: session.id, operation: "abandon" },
                  { onError: showError },
                )
              }
            />
          )}
          {session === undefined ? null : <SessionErrorNotice error={session.error ?? null} />}
          {snapshot.isPending && sessionId !== null ? (
            <div className="flex flex-1 items-center justify-center">
              <Loader2 aria-hidden="true" className="size-4 animate-spin text-muted-foreground" />
            </div>
          ) : session === undefined ? (
            <BeforeConception
              bookId={bookId}
              book={book.data}
              check={latestCheck}
              checkSession={checkSession}
              checkSnapshot={checkSnapshot.data}
              startPending={start.isPending}
              checkPending={startKnowledgeCheck.isPending}
              onStart={() =>
                start.mutate(undefined, {
                  onSuccess: ({ warnings }) => showWarnings(warnings ?? []),
                  onError: showError,
                })
              }
              onCheck={() => startKnowledgeCheck.mutate("", { onError: showError })}
            />
          ) : (
            <ActivityStream
              session={session}
              messages={messages}
              inbox={snapshot.data?.inbox ?? []}
              paneName="conversation"
            />
          )}
          {awaitCall === null ? null : awaitCall.name === "ask_user" ? (
            /*
              A pending form is a fixed block above the composer, so an unbounded one eats the
              stream: with two questions the form is ~370px tall and the conversation pane has no
              room left at all (its `min-h-0` lets it shrink to nothing). The form gets its own
              bounded scroll area instead, which keeps the conversation history readable while the
              author answers.
            */
            <div className="max-h-[45%] shrink-0 overflow-y-auto border-t p-3" data-scroll-pane="form">
              <AskUserForm
                questions={readQuestions(awaitCall.ui)}
                pending={answer.isPending}
                onSubmit={submitAnswer}
              />
            </div>
          ) : (
            <div className="max-h-[45%] shrink-0 overflow-y-auto border-t p-3" data-scroll-pane="form">
              <ReviewRequestForm
                message={
                  typeof awaitCall.ui === "object" && awaitCall.ui !== null
                    ? String((awaitCall.ui as { message?: unknown }).message ?? "")
                    : ""
                }
                items={reviewItems}
                pending={answer.isPending}
                onSubmit={submitAnswer}
              />
            </div>
          )}
          {session === undefined ? null : (
            <Composer
              session={session}
              busy={busy}
              onSend={(text) =>
                send.mutate({ sessionId: session.id, text }, { onError: showError })
              }
              onFeedback={() => undefined}
            />
          )}
        </section>

        {/* Documents */}
        <section className="flex min-h-0 min-w-0 flex-1 flex-col">
          <Tabs value={activeTab} onValueChange={setActiveTab} className="flex min-h-0 flex-1 flex-col">
            <TabsList className="mx-3 mt-2 w-fit">
              <TabsTrigger value="outline">{t("book:conception.tab.outline")}</TabsTrigger>
              <TabsTrigger value="characters">{t("book:conception.tab.characters")}</TabsTrigger>
              <TabsTrigger value="world">{t("book:conception.tab.world")}</TabsTrigger>
              <TabsTrigger value="protagonist">
                {t("book:conception.tab.protagonist")}
              </TabsTrigger>
              <TabsTrigger value="research_topics">
                {t("book:conception.tab.research_topics")}
              </TabsTrigger>
            </TabsList>
            {/* The document column's own scroll container: independent of the conversation pane. */}
            <div
              data-scroll-pane="document"
              className="min-h-0 flex-1 overflow-y-auto p-3"
            >
              {(["outline", "world", "protagonist"] as const).map((kind) => (
                <TabsContent key={kind} value={kind} className="mt-0">
                  <DocumentEditor
                    kind={kind}
                    document={docList.find((doc) => doc.kind === kind)}
                    editable={editable}
                    busy={putOutline.isPending || putWorld.isPending || putProtagonist.isPending}
                    draft={docDraft?.kind === kind ? docDraft : null}
                    onDraft={(content, revision) => setDocDraft({ kind, revision, content })}
                    onSave={(content, revision) => {
                      const target =
                        kind === "outline"
                          ? putOutline
                          : kind === "world"
                            ? putWorld
                            : putProtagonist
                      target.mutate(
                        { content, base_revision: revision, note: "" },
                        { onSuccess: () => setDocDraft(null), onError: showError },
                      )
                    }}
                  />
                </TabsContent>
              ))}
              <TabsContent value="characters" className="mt-0">
                <CharacterRankList
                  characters={characterList}
                  editable={editable}
                  busy={busy}
                  onReorder={(ids) =>
                    reorder.mutate(ids, { onError: showError })
                  }
                  onPatch={(id, patch) =>
                    patchCharacter.mutate(
                      { id, patch: { ...patch, note: "" } },
                      { onError: showError },
                    )
                  }
                  onDelete={(id) => deleteCharacter.mutate(id, { onError: showError })}
                />
              </TabsContent>
              <TabsContent value="research_topics" className="mt-0">
                <ResearchTopicsEditor
                  topics={topicList}
                  editable={editable}
                  busy={busy}
                  onCreate={(input) =>
                    createTopic.mutate({ ...input, book_id: bookId }, { onError: showError })
                  }
                  onPatch={(id, patch) => patchTopic.mutate({ taskId: id, patch }, { onError: showError })}
                  onDelete={(id) => deleteTopic.mutate(id, { onError: showError })}
                />
              </TabsContent>
            </div>
            <footer className="flex items-center justify-end gap-2 border-t px-3 py-2">
              <Button
                size="sm"
                disabled={!complete || busy || book.data.status !== "conceiving"}
                onClick={() => {
                  if (anyUnapproved) {
                    setConfirmFinalize(true)
                  } else {
                    doFinalize()
                  }
                }}
              >
                {t("book:conception.finalize")}
              </Button>
            </footer>
          </Tabs>
        </section>
      </div>
      <ConfirmDialog
        open={confirmFinalize}
        onOpenChange={setConfirmFinalize}
        title={t("book:conception.finalize_unapproved_title")}
        description={t("book:conception.finalize_unapproved_body")}
        pending={finalize.isPending}
        onConfirm={doFinalize}
      />
    </div>
  )
}

/** The conversation pane before a conception session exists (spec/11 §5.4, D-158). */
function BeforeConception({
  bookId,
  book,
  check,
  checkSession,
  checkSnapshot,
  startPending,
  checkPending,
  onStart,
  onCheck,
}: {
  bookId: number
  book: BookDto
  check: ResearchTaskDto | undefined
  checkSession: SessionDto | undefined
  checkSnapshot: { messages: MessageWithCallsDto[]; inbox: unknown[] } | undefined
  startPending: boolean
  checkPending: boolean
  onStart: () => void
  onCheck: () => void
}) {
  const { t } = useTranslation()
  const startButton = (
    <Button
      size="sm"
      variant={book.conception.start_after_knowledge_check ? "outline" : "default"}
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
            {startButton}
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

  if (knowledgeCheckOpen(check)) {
    const session = checkSession
    return (
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-4" data-testid="knowledge-check-open">
        <div className="flex flex-wrap items-center gap-2">
          <h2 className="text-sm font-semibold">{t("book:conception.knowledge_check.title")}</h2>
          {session === undefined ? (
            <StatusBadge status={check.status} variant="canon" />
          ) : (
            <SessionStatusBadge status={session.status} />
          )}
        </div>
        <div className="flex flex-wrap items-center gap-2 text-xs">
          <Link
            to={`/research/${check.id}`}
            className="inline-flex items-center gap-1 text-primary underline-offset-2 hover:underline"
          >
            <ExternalLink className="size-3.5" aria-hidden="true" />
            {t("book:conception.knowledge_check.open_task")}
          </Link>
        </div>
        {session === undefined ? (
          <div className="flex flex-1 items-center justify-center">
            <Loader2 className="size-4 animate-spin text-muted-foreground" aria-hidden="true" />
          </div>
        ) : (
          <div className="flex h-[24rem] flex-col overflow-hidden rounded-md border">
            <ActivityStream
              session={session}
              messages={checkSnapshot?.messages ?? []}
              inbox={[]}
              paneName="knowledge-check"
            />
          </div>
        )}
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

  const terminal = check.status === "done"
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-4" data-testid={`knowledge-check-${check.status}`}>
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-sm font-semibold">{t("book:conception.knowledge_check.title")}</h2>
        {terminal ? null : <StatusBadge status={check.status} variant="canon" />}
        <Link
          to={`/research/${check.id}`}
          className="inline-flex items-center gap-1 text-xs text-primary underline-offset-2 hover:underline"
        >
          <ExternalLink className="size-3.5" aria-hidden="true" />
          {t("book:conception.knowledge_check.open_task")}
        </Link>
      </div>
      {terminal ? (
        <>
          {check.report === "" ? (
            <p className="text-sm text-muted-foreground">
              {t("book:conception.knowledge_check.no_report")}
            </p>
          ) : (
            <div className="prose-cjk rounded-md border p-3 text-sm">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{check.report}</ReactMarkdown>
            </div>
          )}
          <p className="text-xs text-muted-foreground">
            {t("book:conception.knowledge_check.entries_created", {
              count: check.entries_created,
            })}
          </p>
          <Link
            to={`/books/${bookId}/knowledge`}
            className="text-xs text-primary underline-offset-2 hover:underline"
          >
            {t("book:conception.knowledge_check.open_knowledge")}
          </Link>
        </>
      ) : (
        <p className="text-sm text-muted-foreground">
          {t(`agent:status.${check.status}`, { defaultValue: check.status })}
        </p>
      )}
      <div className="mt-auto flex flex-wrap items-center gap-2">
        {startButton}
      </div>
    </div>
  )
}

/** One document tab: a textarea plus save, editable whenever the session is not running. */
function DocumentEditor({
  kind,
  document,
  editable,
  busy,
  draft,
  onDraft,
  onSave,
}: {
  kind: SettingDocKind
  document: { content: string; revision: number } | undefined
  editable: boolean
  busy: boolean
  draft: { revision: number; content: string } | null
  onDraft: (content: string, revision: number) => void
  onSave: (content: string, revision: number) => void
}) {
  const { t } = useTranslation()
  const content =
    document === undefined
      ? ""
      : draft !== null && draft.revision === document.revision
        ? draft.content
        : document.content
  return (
    <div className="flex min-h-full flex-col gap-2">
      <Textarea
        value={content}
        rows={18}
        disabled={!editable}
        className="min-h-[18rem] flex-1 font-mono text-xs"
        placeholder={t(`book:conception.doc_placeholder.${kind}`)}
        onChange={(event) => onDraft(event.target.value, document?.revision ?? 0)}
      />
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs text-muted-foreground">
          {document === undefined
            ? ""
            : t("book:conception.revision", { revision: document.revision })}
        </span>
        <Button
          size="sm"
          variant="outline"
          disabled={!editable || busy || document === undefined || content === document.content}
          onClick={() => onSave(content, document?.revision ?? 0)}
        >
          {busy ? t("common:saving") : t("common:save")}
        </Button>
      </div>
    </div>
  )
}
