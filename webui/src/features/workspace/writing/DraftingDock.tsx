import { type SubmitEvent, useCallback } from "react"
import { useTranslation } from "react-i18next"

import { useChapterName } from "../chapters"
import { CitationField, CitedQuotes } from "../CitationField"
import { QuestionDock } from "../QuestionDock"
import { RunProgress } from "../RunDock"
import { quote } from "./edits"
import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { ChapterView } from "@/api/generated/ChapterView"
import type { Citation } from "@/api/generated/Citation"
import type { FeedbackView } from "@/api/generated/FeedbackView"
import type { HeldCallView } from "@/api/generated/HeldCallView"
import type { RunView } from "@/api/generated/RunView"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import {
  useCancelPause,
  usePauseRun,
  useResumeRun,
  useSetApprovalMode,
  useStopRun,
  useSubmitFeedback,
} from "@/api/queries/workspace"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Field, FieldDescription, FieldError, FieldLabel } from "@/components/ui/field"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { useErrorText } from "@/i18n/errors"
import { InsertTemplate } from "@/features/templates/InsertTemplate"
import { appended, useDraft, useDraftList } from "@/lib/drafts"
import { submitOnModEnter } from "@/lib/keyboard"

/**
 * The dock while the body is written in AutoApprove (§9.3, §18.3): the run's state with
 * pause, continue and stop, and the feedback the writer receives.
 */
export function DraftingDock({
  view,
  chapter,
  run,
}: {
  view: WorkspaceView
  chapter: ChapterView
  run: RunView
}) {
  return (
    <div className="grid gap-6 md:grid-cols-2">
      <div className="flex flex-col items-start gap-3">
        {view.question !== null ? (
          <QuestionDock view={view} question={view.question} />
        ) : run.status === "paused" ? (
          <PausedState view={view} run={run} />
        ) : (
          <WritingState view={view} chapter={chapter} run={run} />
        )}
        <ApprovalModeSwitch view={view} />
      </div>
      <FeedbackPanel view={view} run={run} />
    </div>
  )
}

function WritingState({
  view,
  chapter,
  run,
}: {
  view: WorkspaceView
  chapter: ChapterView
  run: RunView
}) {
  const { t } = useTranslation()
  const chapterName = useChapterName()
  const pause = usePauseRun(view.version.id)
  const cancelPause = useCancelPause(view.version.id)
  const error = pause.error ?? cancelPause.error

  return (
    <>
      <RunProgress view={view} run={run}>
        {t(($) => $.workspace.chapter.drafting, { chapter: chapterName(chapter) })}{" "}
        {t(($) => $.workspace.chapter.words, { count: chapter.body?.wordCount ?? 0 })}
      </RunProgress>
      {view.pauseRequested ? (
        <div className="flex flex-col items-start gap-2">
          <p role="status">{t(($) => $.workspace.writing.pausing)}</p>
          <Button
            variant="outline"
            size="sm"
            disabled={cancelPause.isPending}
            onClick={() => cancelPause.mutate(run.id)}
          >
            {t(($) => $.workspace.writing.cancelPause)}
          </Button>
        </div>
      ) : (
        <Button
          variant="outline"
          size="sm"
          disabled={pause.isPending}
          onClick={() => pause.mutate(run.id)}
        >
          {t(($) => $.workspace.writing.pause)}
        </Button>
      )}
      {error !== null && <ErrorAlert error={error} />}
    </>
  )
}

/** Says what the held call would do. */
function useHeldSummary(): (held: HeldCallView) => string {
  const { t } = useTranslation()
  return useCallback(
    (held: HeldCallView) => {
      const edit = held.edit
      if (edit === null) return t(($) => $.workspace.writing.heldSubmit)
      if (edit.op === "append") return t(($) => $.workspace.writing.heldAppend)
      if (edit.newStr === "") {
        return t(($) => $.workspace.writing.heldDelete, { old: quote(edit.oldStr) })
      }
      return t(($) => $.workspace.writing.heldReplace, {
        old: quote(edit.oldStr),
        new: quote(edit.newStr),
      })
    },
    [t],
  )
}

function PausedState({ view, run }: { view: WorkspaceView; run: RunView }) {
  const { t } = useTranslation()
  const heldSummary = useHeldSummary()
  const resume = useResumeRun(view.version.id)
  const stop = useStopRun(view.version.id)
  const error = resume.error ?? stop.error

  return (
    <>
      <div role="status" className="flex flex-col gap-1">
        <p className="font-medium">{t(($) => $.workspace.writing.paused)}</p>
        {view.held !== null && <p className="text-muted-foreground">{heldSummary(view.held)}</p>}
      </div>
      {error !== null && <ErrorAlert error={error} />}
      <div className="flex flex-wrap gap-2">
        <Button disabled={resume.isPending} onClick={() => resume.mutate(run.id)}>
          {t(($) => $.workspace.writing.continue)}
        </Button>
        <Button variant="outline" disabled={stop.isPending} onClick={() => stop.mutate(run.id)}>
          {t(($) => $.workspace.run.stop)}
        </Button>
      </div>
    </>
  )
}

/** The writer's feedback: what is on its way, and a form for more (§9.2). */
function FeedbackPanel({ view, run }: { view: WorkspaceView; run: RunView }) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const send = useSubmitFeedback(view.version.id)
  const key = `${view.version.id}.feedback.${run.id}`
  const text = useDraft(key, "")
  const citations = useDraftList<Citation>(`${key}.citations`)
  // Feedback while paused, or before a requested pause is held, also lets the run go on.
  const continues = run.status === "paused" || view.pauseRequested

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    send.mutate(
      { runId: run.id, text: text.value, citations: citations.items },
      {
        onSuccess: () => {
          text.clear()
          citations.clear()
        },
      },
    )
  }

  const fieldError = errorForField(send.error, "feedback")
  const formError = errorForForm(send.error, ["feedback"])
  const id = `run-feedback-${run.id}`
  return (
    <div className="flex flex-col gap-3">
      {view.feedback.length > 0 && (
        <section aria-labelledby={`${id}-queue`}>
          <h3 id={`${id}-queue`} className="mb-1 text-sm font-medium">
            {t(($) => $.workspace.writing.queue)}
          </h3>
          <ul className="flex flex-col gap-1 text-sm">
            {view.feedback.map((item) => (
              <FeedbackItem key={item.id} item={item} />
            ))}
          </ul>
        </section>
      )}
      <form onSubmit={submit} className="flex flex-col gap-2">
        <Field data-invalid={fieldError !== undefined}>
          <FieldLabel htmlFor={id}>{t(($) => $.workspace.writing.feedback)}</FieldLabel>
          <Textarea
            id={id}
            value={text.value}
            onChange={(event) => text.setValue(event.target.value)}
            onKeyDown={submitOnModEnter}
            aria-invalid={fieldError !== undefined}
            rows={2}
          />
          <FieldDescription>
            {text.unsaved
              ? t(($) => $.workspace.draft)
              : t(($) => $.workspace.writing.feedbackHint)}
          </FieldDescription>
          {fieldError !== undefined && <FieldError>{errorText(fieldError)}</FieldError>}
        </Field>
        <div className="flex flex-wrap items-start gap-2">
          <CitationField citations={citations} />
          <InsertTemplate onInsert={(inserted) => text.setValue(appended(text.value, inserted))} />
        </div>
        {formError !== undefined && <ErrorAlert error={formError} />}
        <div>
          <Button
            type="submit"
            variant="secondary"
            disabled={send.isPending || text.value.trim() === ""}
          >
            {continues
              ? t(($) => $.workspace.writing.sendAndContinue)
              : t(($) => $.workspace.writing.send)}
          </Button>
        </div>
      </form>
    </div>
  )
}

function FeedbackItem({ item }: { item: FeedbackView }) {
  const { t } = useTranslation()
  const delivery = item.delivery
  const state =
    delivery === null
      ? t(($) => $.workspace.writing.status[item.status])
      : t(($) => $.workspace.writing.delivery[delivery])
  return (
    <li className="flex flex-col rounded-md border bg-background px-3 py-2">
      <span className="whitespace-pre-wrap">{item.text}</span>
      <CitedQuotes citations={item.citations} />
      <span className="text-xs text-muted-foreground">{state}</span>
    </li>
  )
}

/** Switches between applying edits at once and reviewing each (§9.6). */
export function ApprovalModeSwitch({ view }: { view: WorkspaceView }) {
  const { t } = useTranslation()
  const setMode = useSetApprovalMode(view.version.id)
  const id = `approval-mode-${view.version.id}`
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <Switch
          id={id}
          checked={view.approvalMode === "approve_required"}
          disabled={setMode.isPending}
          onCheckedChange={(checked) =>
            setMode.mutate(checked ? "approve_required" : "auto_approve")
          }
        />
        <label htmlFor={id} className="text-sm">
          {t(($) => $.workspace.writing.reviewEachEdit)}
        </label>
      </div>
      {setMode.isError && <ErrorAlert error={setMode.error} />}
    </div>
  )
}
