import { type SubmitEvent, useState } from "react"
import { useTranslation } from "react-i18next"

import { nextMainChapter, useChapterName } from "../chapters"
import { RevisionText } from "../DocumentView"
import { ReviewForm } from "../ReviewForm"
import { DraftingDock } from "./DraftingDock"
import { isHalted } from "../run"
import { RunProblem, RunProgress } from "../RunDock"
import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { ChapterStep } from "@/api/generated/ChapterStep"
import type { ChapterView } from "@/api/generated/ChapterView"
import type { Preview } from "@/api/generated/Preview"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import {
  useAddExtraChapter,
  useCompleteVersion,
  useNextChapter,
  useStartChapter,
} from "@/api/queries/workspace"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Field, FieldDescription, FieldError, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { useErrorText } from "@/i18n/errors"
import { assertNever } from "@/lib/assert"
import { useDraft } from "@/lib/drafts"
import { submitOnModEnter } from "@/lib/keyboard"
import { readPartialObject, textOf } from "@/lib/partialJson"

/** The dock while a chapter is being written (§18.3): one step, one primary action. */
export function ChapterDock({
  view,
  chapter,
  step,
  preview,
}: {
  view: WorkspaceView
  chapter: ChapterView
  step: ChapterStep
  preview: Preview | null
}) {
  const { t } = useTranslation()
  const chapterName = useChapterName()
  const name = chapterName(chapter)
  const run = view.run

  if (run !== null && isHalted(run.status)) return <RunProblem view={view} run={run} />
  switch (step) {
    case "ready":
      return <ReadyDock view={view} chapter={chapter} />
    case "outline_generating":
      return run === null ? null : (
        <div className="flex flex-col gap-4">
          <RunProgress view={view} run={run}>
            {t(($) => $.workspace.chapter.outlining, { chapter: name })}
          </RunProgress>
          <OutlinePreview preview={preview} />
        </div>
      )
    case "outline_reviewing":
      return (
        <div className="flex flex-col gap-4">
          <section>
            <h3 className="mb-2 text-sm font-medium">
              {t(($) => $.workspace.chapter.outline, { chapter: name })}
            </h3>
            <div className="max-h-64 overflow-y-auto rounded-md border bg-background px-4 py-3">
              <RevisionText revision={chapter.outline} empty="" />
            </div>
          </section>
          {chapter.review?.status === "open" && (
            <ReviewForm
              key={chapter.review.id}
              view={view}
              review={chapter.review}
              approveLabel={t(($) => $.workspace.chapter.approveOutline)}
              hint={t(($) => $.workspace.chapter.reviewHint)}
            />
          )}
        </div>
      )
    case "drafting":
      return run === null ? null : <DraftingDock view={view} chapter={chapter} run={run} />
    case "reviewing":
      return (
        <div className="flex flex-col gap-4">
          <section>
            <h3 className="mb-2 text-sm font-medium">
              {t(($) => $.workspace.chapter.summary, { chapter: name })}
            </h3>
            <p className="whitespace-pre-wrap">{chapter.review?.summary}</p>
          </section>
          {chapter.review?.status === "open" && (
            <ReviewForm
              key={chapter.review.id}
              view={view}
              review={chapter.review}
              approveLabel={t(($) => $.workspace.chapter.approveChapter)}
              hint={t(($) => $.workspace.chapter.reviewHint)}
            />
          )}
        </div>
      )
    case "done":
      return <DoneDock view={view} chapter={chapter} />
    default:
      return assertNever(step)
  }
}

/** The outline as it streams in. */
function OutlinePreview({ preview }: { preview: Preview | null }) {
  const { t } = useTranslation()
  const call = preview?.toolCalls.find((item) => item.name === "submit_chapter_outline")
  const content = call === undefined ? undefined : textOf(readPartialObject(call.arguments).content)
  if (content === undefined) return null
  return (
    <section aria-label={t(($) => $.workspace.preview.label)}>
      <p className="mb-2 text-xs text-muted-foreground">{t(($) => $.workspace.preview.note)}</p>
      <p className="max-h-48 overflow-y-auto whitespace-pre-wrap text-muted-foreground">
        {content}
      </p>
    </section>
  )
}

/** The chapter's guidance, then the start of its outline (§7). */
function ReadyDock({ view, chapter }: { view: WorkspaceView; chapter: ChapterView }) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const chapterName = useChapterName()
  const start = useStartChapter(view.version.id)
  const guidance = useDraft(`${view.version.id}.guidance.${chapter.id}`, "")

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    start.mutate(
      { revision: view.revision, guidance: guidance.value },
      { onSuccess: guidance.clear },
    )
  }

  const guidanceError = errorForField(start.error, "chapter_guidance")
  const formError = errorForForm(start.error, ["chapter_guidance"])
  return (
    <form onSubmit={submit} className="flex flex-col gap-3">
      {chapter.goal !== "" && (
        <p className="text-muted-foreground">
          {t(($) => $.workspace.chapter.goal, { goal: chapter.goal })}
        </p>
      )}
      <Field data-invalid={guidanceError !== undefined}>
        <FieldLabel htmlFor="chapter-guidance">
          {t(($) => $.workspace.chapter.guidance, { chapter: chapterName(chapter) })}
        </FieldLabel>
        <Textarea
          id="chapter-guidance"
          value={guidance.value}
          onChange={(event) => guidance.setValue(event.target.value)}
          onKeyDown={submitOnModEnter}
          aria-invalid={guidanceError !== undefined}
          rows={3}
        />
        <FieldDescription>
          {guidance.unsaved
            ? t(($) => $.workspace.draft)
            : t(($) => $.workspace.chapter.guidanceHint)}
        </FieldDescription>
        {guidanceError !== undefined && <FieldError>{errorText(guidanceError)}</FieldError>}
      </Field>
      {formError !== undefined && <ErrorAlert error={formError} />}
      <div>
        <Button type="submit" disabled={start.isPending}>
          {t(($) => $.workspace.chapter.start)}
        </Button>
      </div>
    </form>
  )
}

/** An approved chapter: on to the next one, or completion (§5.3, D05). */
function DoneDock({ view, chapter }: { view: WorkspaceView; chapter: ChapterView }) {
  const { t } = useTranslation()
  const chapterName = useChapterName()
  const next = useNextChapter(view.version.id)
  const complete = useCompleteVersion(view.version.id)
  const following = nextMainChapter(view, chapter)
  const error = next.error ?? complete.error

  return (
    <div className="flex flex-col items-start gap-3">
      <p className="font-medium">
        {t(($) => $.workspace.chapter.done, { chapter: chapterName(chapter) })}
      </p>
      {chapter.kind === "main" && following === undefined && (
        <p className="text-muted-foreground">{t(($) => $.workspace.chapter.completionHint)}</p>
      )}
      {error !== null && <ErrorAlert error={error} />}
      {following !== undefined ? (
        <Button disabled={next.isPending} onClick={() => next.mutate({ revision: view.revision })}>
          {t(($) => $.workspace.chapter.nextChapter)}
        </Button>
      ) : (
        <Button
          disabled={complete.isPending}
          onClick={() => complete.mutate({ revision: view.revision })}
        >
          {chapter.kind === "main"
            ? t(($) => $.workspace.chapter.confirmCompletion)
            : t(($) => $.workspace.chapter.finishExtra)}
        </Button>
      )}
    </div>
  )
}

/** A completed novel, which can take extra chapters (§7, D13). */
export function CompletedDock({ view }: { view: WorkspaceView }) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const add = useAddExtraChapter(view.version.id)
  const [title, setTitle] = useState("")

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    add.mutate({ revision: view.revision, title }, { onSuccess: () => setTitle("") })
  }

  const titleError = errorForField(add.error, "chapter_title")
  const formError = errorForForm(add.error, ["chapter_title"])
  return (
    <form onSubmit={submit} className="flex flex-col gap-3">
      <p className="font-medium">{t(($) => $.workspace.chapter.completed)}</p>
      <Field data-invalid={titleError !== undefined}>
        <FieldLabel htmlFor="extra-chapter-title">
          {t(($) => $.workspace.chapter.extraTitle)}
        </FieldLabel>
        <Input
          id="extra-chapter-title"
          value={title}
          onChange={(event) => setTitle(event.target.value)}
          aria-invalid={titleError !== undefined}
          className="max-w-md"
        />
        <FieldDescription>{t(($) => $.workspace.chapter.extraHint)}</FieldDescription>
        {titleError !== undefined && <FieldError>{errorText(titleError)}</FieldError>}
      </Field>
      {formError !== undefined && <ErrorAlert error={formError} />}
      <div>
        <Button type="submit" disabled={add.isPending}>
          {t(($) => $.workspace.chapter.addExtra)}
        </Button>
      </div>
    </form>
  )
}
