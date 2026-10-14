import { XIcon } from "lucide-react"
import { type SubmitEvent, useState } from "react"
import { useTranslation } from "react-i18next"

import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { PromptTemplateId } from "@/api/generated/PromptTemplateId"
import type { PromptTemplateView } from "@/api/generated/PromptTemplateView"
import type { TemplateVariable } from "@/api/generated/TemplateVariable"
import { useCreateTemplate, useDeleteTemplate, useUpdateTemplate } from "@/api/queries/templates"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { useErrorText } from "@/i18n/errors"

const FIELDS = ["template_title", "template_body", "template_variable_description"] as const

/** Creating a template, or editing and deleting one. */
export function TemplateForm({
  template,
  onSaved,
  onDeleted,
}: {
  template: PromptTemplateView | null
  onSaved: (id: PromptTemplateId) => void
  onDeleted: () => void
}) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const create = useCreateTemplate()
  const update = useUpdateTemplate()
  const remove = useDeleteTemplate()
  const [title, setTitle] = useState(template?.title ?? "")
  const [body, setBody] = useState(template?.body ?? "")
  const [variables, setVariables] = useState<TemplateVariable[]>(template?.variables ?? [])
  const save = template === null ? create : update
  const error = save.error ?? remove.error

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const input = { title, body, variables }
    const done = { onSuccess: (saved: PromptTemplateView) => onSaved(saved.id) }
    if (template === null) create.mutate(input, done)
    else update.mutate({ ...input, template }, done)
  }
  const changeVariable = (index: number, change: Partial<TemplateVariable>) => {
    setVariables((current) =>
      current.map((variable, at) => (at === index ? { ...variable, ...change } : variable)),
    )
  }

  const titleError = errorForField(error, "template_title")
  const bodyError = errorForField(error, "template_body")
  const descriptionError = errorForField(error, "template_variable_description")
  const formError = errorForForm(error, FIELDS)
  return (
    <form onSubmit={submit} className="flex min-w-0 flex-col gap-4">
      <FieldGroup>
        <Field data-invalid={titleError !== undefined}>
          <FieldLabel htmlFor="template-title">{t(($) => $.templates.title)}</FieldLabel>
          <Input
            id="template-title"
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            aria-invalid={titleError !== undefined}
          />
          {titleError !== undefined && <FieldError>{errorText(titleError)}</FieldError>}
        </Field>
        <Field data-invalid={bodyError !== undefined}>
          <FieldLabel htmlFor="template-body">{t(($) => $.templates.body)}</FieldLabel>
          <Textarea
            id="template-body"
            value={body}
            onChange={(event) => setBody(event.target.value)}
            aria-invalid={bodyError !== undefined}
            className="min-h-48"
          />
          <FieldDescription>{t(($) => $.templates.bodyHint)}</FieldDescription>
          {bodyError !== undefined && <FieldError>{errorText(bodyError)}</FieldError>}
        </Field>
        <FieldSet>
          <FieldLegend variant="label">{t(($) => $.templates.variables)}</FieldLegend>
          {variables.map((variable, index) => (
            // Rows have no IDs while they are edited; a row's place is its identity.
            <div key={index} className="grid grid-cols-[10rem_1fr_auto] items-center gap-2">
              <Input
                aria-label={t(($) => $.templates.name)}
                value={variable.name}
                onChange={(event) => changeVariable(index, { name: event.target.value })}
                className="font-mono"
              />
              <Input
                aria-label={t(($) => $.templates.variableDescription)}
                value={variable.description}
                onChange={(event) => changeVariable(index, { description: event.target.value })}
              />
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={t(($) => $.templates.removeVariable, { name: variable.name })}
                onClick={() => setVariables((current) => current.filter((_, at) => at !== index))}
              >
                <XIcon />
              </Button>
            </div>
          ))}
          {descriptionError !== undefined && <FieldError>{errorText(descriptionError)}</FieldError>}
          <div>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => setVariables((current) => [...current, { name: "", description: "" }])}
            >
              {t(($) => $.templates.addVariable)}
            </Button>
          </div>
        </FieldSet>
      </FieldGroup>
      {formError !== undefined && <ErrorAlert error={formError} />}
      <div className="flex flex-wrap items-center gap-2">
        <Button type="submit" disabled={save.isPending}>
          {template === null ? t(($) => $.templates.create) : t(($) => $.templates.save)}
        </Button>
        {template !== null && (
          <AlertDialog>
            <AlertDialogTrigger render={<Button type="button" variant="ghost" />}>
              {t(($) => $.templates.delete)}
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>
                  {t(($) => $.templates.deleteTitle, { title: template.title })}
                </AlertDialogTitle>
                <AlertDialogDescription>{t(($) => $.templates.deleteBody)}</AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>{t(($) => $.common.actions.cancel)}</AlertDialogCancel>
                <AlertDialogAction
                  variant="destructive"
                  disabled={remove.isPending}
                  onClick={() => remove.mutate(template, { onSuccess: onDeleted })}
                >
                  {t(($) => $.templates.deleteConfirm)}
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        )}
        {save.isSuccess && (
          <span className="text-sm text-muted-foreground">{t(($) => $.templates.saved)}</span>
        )}
      </div>
    </form>
  )
}
