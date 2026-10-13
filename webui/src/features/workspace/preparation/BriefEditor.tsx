import type { SubmitEvent } from "react"
import { useTranslation } from "react-i18next"

import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useSaveBrief } from "@/api/queries/workspace"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Field, FieldDescription, FieldError, FieldLabel } from "@/components/ui/field"
import { Textarea } from "@/components/ui/textarea"
import { useErrorText } from "@/i18n/errors"
import type { Draft } from "@/lib/drafts"
import { submitOnModEnter } from "@/lib/keyboard"

/** Writing the novel brief before preparation starts (§6). */
export function BriefEditor({ view, brief }: { view: WorkspaceView; brief: Draft }) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const save = useSaveBrief(view.version.id)

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    save.mutate({ revision: view.revision, brief: brief.value }, { onSuccess: brief.clear })
  }

  const briefError = errorForField(save.error, "novel_brief")
  const formError = errorForForm(save.error, ["novel_brief"])

  return (
    <form onSubmit={submit} className="flex flex-col gap-4">
      <Field data-invalid={briefError !== undefined}>
        <FieldLabel htmlFor="novel-brief" className="font-heading text-2xl font-semibold">
          {t(($) => $.workspace.brief.label)}
        </FieldLabel>
        <FieldDescription>{t(($) => $.workspace.brief.description)}</FieldDescription>
        <Textarea
          id="novel-brief"
          value={brief.value}
          onChange={(event) => brief.setValue(event.target.value)}
          onKeyDown={submitOnModEnter}
          aria-invalid={briefError !== undefined}
          className="min-h-[50svh] font-reading text-lg leading-[1.85] md:text-lg"
        />
        {briefError !== undefined && <FieldError>{errorText(briefError)}</FieldError>}
      </Field>
      {brief.conflict && brief.unsaved && (
        <p role="alert" className="text-sm text-destructive">
          {t(($) => $.workspace.brief.conflict)}
        </p>
      )}
      {formError !== undefined && <ErrorAlert error={formError} />}
      <div className="flex items-center gap-3">
        <Button type="submit" disabled={save.isPending || !brief.unsaved}>
          {save.isPending ? t(($) => $.common.status.saving) : t(($) => $.workspace.brief.save)}
        </Button>
        <span className="text-sm text-muted-foreground">
          {brief.unsaved
            ? t(($) => $.workspace.draft)
            : view.brief !== null && t(($) => $.workspace.brief.saved)}
        </span>
      </div>
    </form>
  )
}
