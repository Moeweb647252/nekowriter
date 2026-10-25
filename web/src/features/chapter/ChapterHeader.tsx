/** The chapter header: label, inline title, status, word count and status actions. */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useQueryClient } from "@tanstack/react-query"

import { useApproveChapter, useSubmitChapter } from "@/api/hooks/chapters"
import {
  useMaintainChapter,
  useResyncChapter,
  useReviews,
  useStartReview,
} from "@/api/hooks/reviews"
import { WriteDialog } from "@/features/chapter/WriteDialog"
import { SendBackDialog } from "@/features/chapter/SendBackDialog"
import { ConfirmDialog } from "@/components/common/ConfirmDialog"
import { useErrorToast, useWarningToasts } from "@/components/common/ErrorToast"
import { StatusBadge } from "@/components/common/StatusBadge"
import { WordCount } from "@/components/common/WordCount"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import type { BookSettings } from "@/api/gen/BookSettings"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import { keys } from "@/api/keys"
import { isComposing } from "@/lib/hotkeys"

export type ChapterHeaderProps = {
  chapter: ChapterDto
  settings: BookSettings
  language: string
  onRename: (title: string) => void
  /** Called with a new session id (writer, reviewer or archivist) so the panel can focus it. */
  onSessionStarted?: (sessionId: number) => void
}

/** Name/status/actions of one chapter. */
export function ChapterHeader({
  chapter,
  settings,
  language,
  onRename,
  onSessionStarted,
}: ChapterHeaderProps) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const showError = useErrorToast()
  const showWarnings = useWarningToasts()
  const submit = useSubmitChapter(chapter.id, chapter.book_id)
  const approve = useApproveChapter(chapter.id, chapter.book_id)
  const reviews = useReviews(chapter.id)
  const startReview = useStartReview(chapter.id, chapter.book_id)
  const maintain = useMaintainChapter(chapter.id, chapter.book_id)
  const resync = useResyncChapter(chapter.id, chapter.book_id)
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(chapter.title)
  const [writeOpen, setWriteOpen] = useState(false)
  const [sendBackOpen, setSendBackOpen] = useState(false)
  const [submitConfirmOpen, setSubmitConfirmOpen] = useState(false)

  const save = () => {
    setEditing(false)
    const trimmed = draft.trim()
    if (trimmed !== chapter.title) {
      onRename(trimmed)
    }
  }

  // Status actions from PROJECT §8.5; drafting/revising are driven by the agent panel.
  const canSubmit = chapter.status === "drafting" || chapter.status === "revising"
  const canApprove = chapter.status === "in_review"
  const latestReport = reviews.data?.reports[0]
  const canMaintain =
    chapter.status === "official" &&
    (chapter.canon_status === "pending" || chapter.canon_status === "failed")
  const canResync = chapter.status === "official" && chapter.canon_status === "stale"
  // spec/08 §1.1: a manual submit ends the chapter's open, non-running writer session, so the
  // author is warned first. A `running` session is left to the endpoint's 409: stopping it is a
  // separate action on the session.
  const openWriterSession = chapter.open_sessions.find(
    (session) => session.role === "writer" && session.status !== "running",
  )
  const runSubmit = () => {
    submit.mutate(undefined, {
      onSuccess: () => {
        setSubmitConfirmOpen(false)
        void queryClient.invalidateQueries({ queryKey: keys.chapters.reviews(chapter.id) })
      },
      onError: (error) => {
        setSubmitConfirmOpen(false)
        showError(error)
      },
    })
  }
  const requestSubmit = () => {
    if (openWriterSession === undefined) {
      runSubmit()
    } else {
      setSubmitConfirmOpen(true)
    }
  }

  return (
    <header className="space-y-2 border-b px-6 py-4">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <span>{t("chapter:label", { number: chapter.number })}</span>
        <StatusBadge status={chapter.status} />
        {chapter.status === "official" ? (
          <StatusBadge status={chapter.canon_status} variant="canon" />
        ) : null}
      </div>
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0 flex-1">
          {editing ? (
            <Input
              value={draft}
              autoFocus
              className="h-8 text-lg font-semibold"
              onChange={(event) => setDraft(event.target.value)}
              onBlur={save}
              onKeyDown={(event) => {
                if (isComposing(event)) {
                  return
                }
                if (event.key === "Enter") {
                  save()
                }
                if (event.key === "Escape") {
                  setDraft(chapter.title)
                  setEditing(false)
                }
              }}
            />
          ) : (
            <button
              type="button"
              className="block max-w-full truncate text-left text-lg font-semibold hover:underline"
              onClick={() => {
                setDraft(chapter.title)
                setEditing(true)
              }}
            >
              {chapter.title || t("chapter:title_placeholder")}
            </button>
          )}
        </div>
        <WordCount
          words={chapter.word_count}
          targetMin={settings.target_words_min}
          targetMax={settings.target_words_max}
          language={language}
          className="pt-1.5"
        />
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {chapter.status === "planned" ? (
          <Button size="sm" disabled={writeOpen} onClick={() => setWriteOpen(true)}>
            {t("chapter:write")}
          </Button>
        ) : null}
        {canSubmit ? (
          <Button
            size="sm"
            variant="outline"
            disabled={submit.isPending || chapter.content.paragraphs.length === 0}
            onClick={requestSubmit}
          >
            {t("chapter:submit")}
          </Button>
        ) : null}
        {canApprove ? (
          <>
            <Button
              size="sm"
              disabled={approve.isPending}
              onClick={() =>
                approve.mutate(undefined, {
                  onSuccess: ({ warnings, maintenance_session_id }) => {
                    showWarnings(warnings)
                    if (maintenance_session_id !== undefined && maintenance_session_id !== null) {
                      onSessionStarted?.(maintenance_session_id)
                    }
                  },
                  onError: showError,
                })
              }
            >
              {t("chapter:approve")}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={chapter.content.paragraphs.length === 0}
              onClick={() => setSendBackOpen(true)}
            >
              {t("chapter:send_back")}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={startReview.isPending}
              onClick={() =>
                startReview.mutate(undefined, {
                  onSuccess: (response) => onSessionStarted?.(response.session_id),
                  onError: showError,
                })
              }
            >
              {t("chapter:review.ask_reviewer")}
            </Button>
          </>
        ) : null}
        {canMaintain ? (
          <Button
            size="sm"
            variant="outline"
            disabled={maintain.isPending}
            onClick={() =>
              maintain.mutate(undefined, {
                onSuccess: (response) => onSessionStarted?.(response.session_id),
                onError: showError,
              })
            }
          >
            {chapter.canon_status === "failed"
              ? t("chapter:review.retry_maintenance")
              : t("chapter:maintain")}
          </Button>
        ) : null}
        {canResync ? (
          <Button
            size="sm"
            variant="outline"
            disabled={resync.isPending}
            onClick={() =>
              resync.mutate(undefined, {
                onSuccess: (response) => onSessionStarted?.(response.session_id),
                onError: showError,
              })
            }
          >
            {t("chapter:resync")}
          </Button>
        ) : null}
      </div>
      <SendBackDialog
        chapterId={chapter.id}
        bookId={chapter.book_id}
        open={sendBackOpen}
        onOpenChange={setSendBackOpen}
        report={latestReport}
        onSentBack={(sessionId) => onSessionStarted?.(sessionId)}
      />
      <WriteDialog
        chapterId={chapter.id}
        bookId={chapter.book_id}
        settings={settings}
        guidance={chapter.guidance}
        open={writeOpen}
        onOpenChange={setWriteOpen}
        onStarted={(sessionId) => onSessionStarted?.(sessionId)}
      />
      <ConfirmDialog
        open={submitConfirmOpen}
        onOpenChange={setSubmitConfirmOpen}
        title={t("chapter:submit_confirm.title")}
        description={t("chapter:submit_confirm.description")}
        confirmLabel={t("chapter:submit_confirm.confirm")}
        destructive={false}
        pending={submit.isPending}
        onConfirm={runSubmit}
      />
    </header>
  )
}
