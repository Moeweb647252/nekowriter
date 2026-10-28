/**
 * The chapter Review tab (PROJECT §8.5, spec/08 §3): reports with their issues, an editable
 * feedback per report, send-back (revise / redraft) and discuss (re-review).
 *
 * Issues jump to the paragraph they quote; the report records the version it reviewed, so a report
 * for an earlier version is labelled as such (spec/08 §1.2).
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { ChevronDown, ChevronRight, MessageSquare, Send, Sparkles } from "lucide-react"

import { useRereview, useReviews, useStartReview } from "@/api/hooks/reviews"
import { useErrorToast } from "@/components/common/ErrorToast"
import { EmptyState } from "@/components/common/EmptyState"
import { ErrorToastInline } from "@/features/settings/ErrorToastInline"
import { SendBackDialog } from "@/features/chapter/SendBackDialog"
import { OutlineDialog } from "@/features/chapter/OutlineDialog"
import { firstPid, reportIsCurrent } from "@/features/chapter/reviewUtils"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Skeleton } from "@/components/ui/skeleton"
import { Textarea } from "@/components/ui/textarea"
import { formatRelativeTime } from "@/lib/format"
import type { BookSettings } from "@/api/gen/BookSettings"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { ReviewReportDto } from "@/api/gen/ReviewReportDto"

export type ReviewTabProps = {
  chapter: ChapterDto
  bookId: number
  /** Called with the session id the API started or continued (so the panel can focus it). */
  onSessionStarted?: (sessionId: number) => void
  /** Jumps to a pid in the Text tab (issues quote paragraphs). */
  onFocusPid?: (pid: string) => void
  settings: BookSettings
}

export function ReviewTab({
  chapter,
  bookId,
  onSessionStarted,
  onFocusPid,
  settings,
}: ReviewTabProps) {
  const { t, i18n } = useTranslation()
  const showError = useErrorToast()
  const reviews = useReviews(chapter.id)
  const startReview = useStartReview(chapter.id, bookId)
  const [sendBack, setSendBack] = useState<{ report?: ReviewReportDto; feedback: string } | null>(
    null,
  )
  const [discuss, setDiscuss] = useState<ReviewReportDto | null>(null)
  const [outlineOpen, setOutlineOpen] = useState(false)

  if (reviews.isPending) {
    return <Skeleton className="h-48" />
  }
  if (reviews.isError) {
    return (
      <div className="p-2">
        <ErrorToastInline message={t("common:error_body")} />
      </div>
    )
  }

  const reports = reviews.data?.reports ?? []
  const feedback = reviews.data?.feedback ?? []
  const latest = reports[0]
  const inReview = chapter.status === "in_review"

  return (
    <div className="mx-auto max-w-[52em] space-y-4">
      <header className="flex flex-wrap items-center gap-2">
        <p className="text-sm text-muted-foreground">
          {inReview
            ? t("chapter:review.waiting", { number: chapter.number })
            : t("chapter:review.not_in_review")}
        </p>
        <div className="ml-auto flex flex-wrap items-center gap-2">
          {inReview ? (
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
              <Sparkles className="size-3.5" />
              {t("chapter:review.ask_reviewer")}
            </Button>
          ) : null}
          {latest === undefined ? null : (
            <Button size="sm" variant="outline" onClick={() => setDiscuss(latest)}>
              <MessageSquare className="size-3.5" />
              {t("chapter:review.discuss")}
            </Button>
          )}
        </div>
      </header>

      {reports.length === 0 ? (
        <EmptyState
          title={t("chapter:review.no_reports_title")}
          description={t("chapter:review.no_reports_body")}
        />
      ) : (
        <ul className="space-y-3">
          {reports.map((report, index) => (
            <li key={report.id}>
              {index === 0 ? (
                <ReportCard
                  report={report}
                  current={reportIsCurrent(chapter.content_version, report)}
                  onSendBack={(editedFeedback) =>
                    setSendBack({ report, feedback: editedFeedback })
                  }
                  onDiscuss={() => setDiscuss(report)}
                  onFocusPid={onFocusPid}
                  language={i18n.language}
                />
              ) : (
                <EarlierReport
                  report={report}
                  current={reportIsCurrent(chapter.content_version, report)}
                  onFocusPid={onFocusPid}
                  language={i18n.language}
                />
              )}
            </li>
          ))}
        </ul>
      )}

      {feedback.length === 0 ? null : (
        <section className="space-y-2">
          <h2 className="text-sm font-medium">{t("chapter:review.feedback_history")}</h2>
          <ul className="divide-y rounded-md border text-xs">
            {feedback.map((item) => (
              <li key={item.id} className="space-y-1 px-3 py-2">
                <div className="flex flex-wrap items-center gap-2 text-muted-foreground">
                  <Badge variant="outline">
                    {t(`chapter:review.mode.${item.mode}`)}
                  </Badge>
                  <span>
                    {t("chapter:review.earlier_version", { version: item.content_version })}
                  </span>
                  <span className="ml-auto">
                    {formatRelativeTime(item.created_at, i18n.language)}
                  </span>
                </div>
                <p className="whitespace-pre-wrap">{item.text}</p>
                {item.issues.length === 0 ? null : (
                  <p className="text-muted-foreground">
                    {t("chapter:review.feedback_issues", { count: item.issues.length })}
                  </p>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      <SendBackDialog
        chapterId={chapter.id}
        bookId={bookId}
        open={sendBack !== null}
        onOpenChange={(open) => {
          if (!open) {
            setSendBack(null)
          }
        }}
        report={sendBack?.report}
        initialFeedback={sendBack?.feedback}
        onSentBack={(sessionId) => {
          setSendBack(null)
          onSessionStarted?.(sessionId)
        }}
        onWriteOutline={() => setOutlineOpen(true)}
      />
      <OutlineDialog
        chapterId={chapter.id}
        bookId={bookId}
        settings={settings}
        guidance={chapter.guidance}
        open={outlineOpen}
        onOpenChange={setOutlineOpen}
        onStarted={(sessionId) => {
          setOutlineOpen(false)
          onSessionStarted?.(sessionId)
        }}
      />
      <DiscussDialog
        report={discuss}
        chapterId={chapter.id}
        bookId={bookId}
        onOpenChange={(open) => {
          if (!open) {
            setDiscuss(null)
          }
        }}
        onStarted={(sessionId) => {
          setDiscuss(null)
          onSessionStarted?.(sessionId)
        }}
      />
    </div>
  )
}

/** One issue: severity, category, pid buttons, quote and the problem + suggestion. */
function IssueLine({
  issue,
  onFocusPid,
}: {
  issue: ReviewReportDto["issues"][number]
  onFocusPid?: (pid: string) => void
}) {
  const { t } = useTranslation()
  const pid = firstPid(issue)
  return (
    <li className="flex flex-wrap items-baseline gap-x-1.5 gap-y-0.5 text-xs">
      <span className="font-medium">{t(`chapter:review.severity.${issue.severity}`)}</span>
      <span className="text-muted-foreground">
        {t(`chapter:review.category.${issue.category}`)}
      </span>
      <span className="flex flex-wrap gap-1">
        {issue.pids.map((item) => (
          <button
            key={item}
            type="button"
            className="rounded bg-muted px-1 font-mono text-[0.7rem] hover:bg-accent"
            onClick={() => onFocusPid?.(item)}
            disabled={pid === undefined || onFocusPid === undefined}
          >
            {item}
          </button>
        ))}
      </span>
      {issue.quote === "" ? null : <span>“{issue.quote}”</span>}
      <span>{issue.problem}</span>
      {issue.suggestion === "" ? null : (
        <span className="text-muted-foreground">→ {issue.suggestion}</span>
      )}
    </li>
  )
}

const REPORT_STRINGS = "chapter:review"

/** The newest report, expanded, with an editable feedback box. */
function ReportCard({
  report,
  current,
  onSendBack,
  onDiscuss,
  onFocusPid,
  language,
}: {
  report: ReviewReportDto
  current: boolean
  onSendBack: (feedback: string) => void
  onDiscuss: () => void
  onFocusPid?: (pid: string) => void
  language: string
}) {
  const { t } = useTranslation()
  const [feedback, setFeedback] = useState(report.feedback)
  return (
    <section className="space-y-3 rounded-md border p-3">
      <header className="flex flex-wrap items-center gap-2">
        <h2 className="text-sm font-medium">
          {t(`${REPORT_STRINGS}.report_title`, { attempt: report.attempt })}
        </h2>
        <Badge variant={report.verdict === "pass" ? "secondary" : "outline"}>
          {t(`${REPORT_STRINGS}.verdict.${report.verdict}`)}
        </Badge>
        {current ? null : (
          <Badge variant="outline" className="border-amber-500/50">
            {t(`${REPORT_STRINGS}.stale_report`)}
          </Badge>
        )}
        <span className="ml-auto text-xs text-muted-foreground">
          {formatRelativeTime(report.created_at, language)}
        </span>
      </header>
      {report.summary === "" ? null : (
        <p className="text-sm whitespace-pre-wrap">{report.summary}</p>
      )}
      {report.user_comment === "" ? null : (
        <p className="text-xs text-muted-foreground">
          {t(`${REPORT_STRINGS}.user_comment`, { comment: report.user_comment })}
        </p>
      )}
      {report.issues.length === 0 ? null : (
        <div>
          <div className="mb-1 text-xs font-medium">
            {t(`${REPORT_STRINGS}.issues_title`, { count: report.issues.length })}
          </div>
          <ul className="space-y-1">
            {report.issues.map((issue, index) => (
              <IssueLine key={index} issue={issue} onFocusPid={onFocusPid} />
            ))}
          </ul>
        </div>
      )}
      <div className="space-y-1.5">
        <label htmlFor="review-feedback" className="text-xs font-medium">
          {t(`${REPORT_STRINGS}.feedback_label`)}
        </label>
        <Textarea
          id="review-feedback"
          rows={4}
          value={feedback}
          onChange={(event) => setFeedback(event.target.value)}
        />
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          disabled={feedback.trim() === ""}
          onClick={() => onSendBack(feedback)}
        >
          <Send className="size-3.5" />
          {t(`${REPORT_STRINGS}.send_back_with_feedback`)}
        </Button>
        <Button size="sm" variant="outline" onClick={onDiscuss}>
          <MessageSquare className="size-3.5" />
          {t(`${REPORT_STRINGS}.discuss`)}
        </Button>
      </div>
    </section>
  )
}

/** An older report, collapsed (PROJECT §8.5 "Earlier: Report #1 …"). */
function EarlierReport({
  report,
  current,
  onFocusPid,
  language,
}: {
  report: ReviewReportDto
  current: boolean
  onFocusPid?: (pid: string) => void
  language: string
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <Collapsible open={open} onOpenChange={setOpen} className="rounded-md border">
      <CollapsibleTrigger asChild>
        <Button
          variant="ghost"
          className="h-auto w-full justify-start gap-2 px-3 py-2 text-left font-normal"
        >
          {open ? (
            <ChevronDown className="size-3.5 shrink-0" />
          ) : (
            <ChevronRight className="size-3.5 shrink-0" />
          )}
          <span className="text-xs">
            {t(`${REPORT_STRINGS}.earlier_report`, { attempt: report.attempt })}
          </span>
          <Badge variant="outline" className="ml-1">
            {t(`${REPORT_STRINGS}.verdict.${report.verdict}`)}
          </Badge>
          {current ? null : (
            <Badge variant="outline" className="border-amber-500/50">
              {t(`${REPORT_STRINGS}.stale_report`)}
            </Badge>
          )}
          <span className="ml-auto text-xs text-muted-foreground">
            {formatRelativeTime(report.created_at, language)}
          </span>
        </Button>
      </CollapsibleTrigger>
      <CollapsibleContent className="space-y-2 border-t px-3 py-2">
        <p className="text-xs whitespace-pre-wrap">{report.summary}</p>
        <ul className="space-y-1">
          {report.issues.map((issue, index) => (
            <IssueLine key={index} issue={issue} onFocusPid={onFocusPid} />
          ))}
        </ul>
      </CollapsibleContent>
    </Collapsible>
  )
}

/** Discuss: a comment becomes a re-review (spec/03 §6). */
function DiscussDialog({
  report,
  chapterId,
  bookId,
  onOpenChange,
  onStarted,
}: {
  report: ReviewReportDto | null
  chapterId: number
  bookId: number
  onOpenChange: (open: boolean) => void
  onStarted: (sessionId: number) => void
}) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const rereview = useRereview(chapterId, bookId)
  const [comment, setComment] = useState("")
  return (
    <Dialog
      open={report !== null}
      onOpenChange={(open) => {
        if (!open) {
          setComment("")
        }
        onOpenChange(open)
      }}
    >
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t("chapter:discuss_dialog.title")}</DialogTitle>
          <DialogDescription>{t("chapter:discuss_dialog.description")}</DialogDescription>
        </DialogHeader>
        <Textarea
          rows={5}
          value={comment}
          placeholder={t("chapter:discuss_dialog.placeholder")}
          onChange={(event) => setComment(event.target.value)}
        />
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          <Button
            disabled={comment.trim() === "" || rereview.isPending || report === null}
            onClick={() => {
              if (report === null) {
                return
              }
              rereview.mutate(
                { reportId: report.id, comment },
                {
                  onSuccess: (response) => {
                    setComment("")
                    onStarted(response.session_id)
                  },
                  onError: showError,
                },
              )
            }}
          >
            {t("chapter:discuss_dialog.send")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
