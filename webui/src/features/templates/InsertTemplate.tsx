import { FileTextIcon } from "lucide-react"
import { type SubmitEvent, useState } from "react"
import { useTranslation } from "react-i18next"

import type { PromptTemplateId } from "@/api/generated/PromptTemplateId"
import { useRenderTemplate } from "@/api/queries/prompts"
import { useTemplates } from "@/api/queries/templates"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog"
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

/**
 * Fills in one of the user's templates and hands the text to `onInsert`, which adds it to
 * a draft (§13.2). The text becomes the user's own words; it is never a prompt.
 */
export function InsertTemplate({ onInsert }: { onInsert: (text: string) => void }) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger render={<Button type="button" variant="outline" size="sm" />}>
        <FileTextIcon />
        {t(($) => $.templates.insert.open)}
      </DialogTrigger>
      <DialogContent>
        {/* Mounted only while the dialog is open, so it loads and starts afresh each time. */}
        <InsertTemplateForm
          onInsert={(text) => {
            onInsert(text)
            setOpen(false)
          }}
        />
      </DialogContent>
    </Dialog>
  )
}

function InsertTemplateForm({ onInsert }: { onInsert: (text: string) => void }) {
  const { t } = useTranslation()
  const templates = useTemplates()
  const render = useRenderTemplate()
  const [chosen, setChosen] = useState<PromptTemplateId | null>(null)
  const [values, setValues] = useState<Record<string, string>>({})
  const template = templates.data?.find((item) => item.id === chosen)
  const items = (templates.data ?? []).map((item) => ({ value: item.id, label: item.title }))

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (template === undefined) return
    const filled = Object.fromEntries(
      template.variables.map(({ name }) => [name, values[name] ?? ""]),
    )
    render.mutate(
      { template: template.body, values: filled },
      { onSuccess: (rendered) => onInsert(rendered.text) },
    )
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-4">
      <DialogHeader>
        <DialogTitle>{t(($) => $.templates.insert.title)}</DialogTitle>
        {items.length === 0 && templates.isSuccess && (
          <DialogDescription>{t(($) => $.templates.insert.none)}</DialogDescription>
        )}
      </DialogHeader>
      {templates.isError && (
        <ErrorAlert error={templates.error} onRetry={() => void templates.refetch()} />
      )}
      {items.length > 0 && (
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="insert-template">{t(($) => $.templates.insert.choose)}</FieldLabel>
            <Select
              items={items}
              value={chosen}
              onValueChange={(value: PromptTemplateId | null) => setChosen(value)}
            >
              <SelectTrigger id="insert-template" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {items.map((item) => (
                  <SelectItem key={item.value} value={item.value}>
                    {item.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
          {template?.variables.map(({ name, description }) => (
            <Field key={name}>
              <FieldLabel htmlFor={`insert-template-${name}`}>{name}</FieldLabel>
              <Input
                id={`insert-template-${name}`}
                value={values[name] ?? ""}
                onChange={(event) =>
                  setValues((current) => ({ ...current, [name]: event.target.value }))
                }
              />
              {description !== "" && <FieldDescription>{description}</FieldDescription>}
            </Field>
          ))}
        </FieldGroup>
      )}
      {render.isError && <ErrorAlert error={render.error} />}
      <DialogFooter>
        <Button type="submit" disabled={template === undefined || render.isPending}>
          {t(($) => $.templates.insert.submit)}
        </Button>
      </DialogFooter>
    </form>
  )
}
