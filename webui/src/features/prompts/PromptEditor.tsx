import { type SubmitEvent, useState } from "react"
import { useTranslation } from "react-i18next"

import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { ProjectId } from "@/api/generated/ProjectId"
import type { PromptView } from "@/api/generated/PromptView"
import type { PromptsView } from "@/api/generated/PromptsView"
import { useRenderTemplate, useSavePrompt } from "@/api/queries/prompts"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Field, FieldDescription, FieldError, FieldLabel } from "@/components/ui/field"
import { Textarea } from "@/components/ui/textarea"
import { useErrorText } from "@/i18n/errors"
import { useDraft } from "@/lib/drafts"
import { submitOnModEnter } from "@/lib/keyboard"

type Source = "builtin" | "system" | "project"

/** The prompt in effect at a scope, and where it comes from. */
function effective(prompt: PromptView, project: ProjectId | null): [string, Source] {
  if (project !== null && prompt.project !== null) return [prompt.project, "project"]
  if (prompt.system !== null) return [prompt.system, "system"]
  return [prompt.builtin, "builtin"]
}

/**
 * Editing one prompt at one scope. Saving stores an override at that scope; resetting
 * deletes it, so the next level applies. The draft is kept in this browser (§18.5).
 */
export function PromptEditor({
  view,
  prompt,
  project,
}: {
  view: PromptsView
  prompt: PromptView
  project: ProjectId | null
}) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const save = useSavePrompt(project)
  const render = useRenderTemplate()
  const [text, source] = effective(prompt, project)
  const draft = useDraft(`prompts.${project ?? "system"}.${prompt.key}.${prompt.role}`, text)
  const [previewed, setPreviewed] = useState<string | null>(null)
  // An override at this scope can be reset to the level below it.
  const ownSource: Source = project === null ? "system" : "project"
  const fallback: Source = project !== null && prompt.system !== null ? "system" : "builtin"

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    save.mutate(
      { key: prompt.key, role: prompt.role, content: draft.value, revision: view.revision },
      { onSuccess: draft.clear },
    )
  }
  const reset = () => {
    save.mutate(
      { key: prompt.key, role: prompt.role, content: null, revision: view.revision },
      { onSuccess: draft.clear },
    )
  }
  const preview = () => {
    const values = Object.fromEntries(
      prompt.variables.map(({ variable }) => [
        variable,
        t(($) => $.prompts.variable[variable].example),
      ]),
    )
    render.mutate(
      { template: draft.value, values },
      { onSuccess: (rendered) => setPreviewed(rendered.text) },
    )
  }

  const id = `prompt-${prompt.key}-${prompt.role}`
  const fieldError = errorForField(save.error, "prompt_content")
  const formError = errorForForm(save.error, ["prompt_content"])
  return (
    <div className="grid gap-6 lg:grid-cols-[1fr_16rem]">
      <form onSubmit={submit} className="flex min-w-0 flex-col gap-3">
        <p role="status" className="text-sm">
          {t(($) => $.prompts.inEffect, { source: t(($) => $.prompts.source[source]) })}
        </p>
        <Field data-invalid={fieldError !== undefined || formError !== undefined}>
          <FieldLabel htmlFor={id}>{t(($) => $.prompts.text)}</FieldLabel>
          <Textarea
            id={id}
            value={draft.value}
            onChange={(event) => {
              draft.setValue(event.target.value)
              setPreviewed(null)
            }}
            onKeyDown={submitOnModEnter}
            className="min-h-96 font-mono text-sm"
          />
          <FieldDescription>
            {draft.unsaved ? t(($) => $.workspace.draft) : t(($) => $.prompts.textHint)}
          </FieldDescription>
          {fieldError !== undefined && <FieldError>{errorText(fieldError)}</FieldError>}
        </Field>
        {draft.conflict && draft.unsaved && (
          <p role="alert" className="text-sm text-destructive">
            {t(($) => $.prompts.conflict)}
          </p>
        )}
        {formError !== undefined && <ErrorAlert error={formError} />}
        <div className="flex flex-wrap items-center gap-2">
          <Button type="submit" disabled={save.isPending || !draft.unsaved}>
            {save.isPending ? t(($) => $.common.status.saving) : t(($) => $.prompts.save)}
          </Button>
          {source === ownSource && (
            <Button type="button" variant="outline" disabled={save.isPending} onClick={reset}>
              {t(($) => $.prompts.reset, { source: t(($) => $.prompts.source[fallback]) })}
            </Button>
          )}
          <Button type="button" variant="ghost" disabled={render.isPending} onClick={preview}>
            {t(($) => $.prompts.preview)}
          </Button>
        </div>
        {render.isError && <ErrorAlert error={render.error} />}
        {previewed !== null && (
          <section className="rounded-md border bg-muted/40 p-4">
            <h3 className="mb-2 text-sm font-medium">{t(($) => $.prompts.previewTitle)}</h3>
            <p className="text-sm whitespace-pre-wrap">{previewed}</p>
          </section>
        )}
      </form>
      <aside className="flex flex-col gap-3">
        <h3 className="text-sm font-medium">{t(($) => $.prompts.variables)}</h3>
        <ul className="flex flex-col gap-3 text-sm">
          {prompt.variables.map(({ variable, required }) => (
            <li key={variable} className="flex flex-col gap-1">
              <div className="flex flex-wrap items-center gap-2">
                <code className="font-mono">{`{{${variable}}}`}</code>
                <Badge variant={required ? "default" : "outline"}>
                  {required ? t(($) => $.prompts.required) : t(($) => $.prompts.optional)}
                </Badge>
              </div>
              <p className="text-muted-foreground">{t(($) => $.prompts.variable[variable].help)}</p>
              <p className="text-muted-foreground">
                {t(($) => $.prompts.example, {
                  example: t(($) => $.prompts.variable[variable].example),
                })}
              </p>
            </li>
          ))}
        </ul>
      </aside>
    </div>
  )
}
