import { type SubmitEvent, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { WritingLanguage } from "@/api/generated/WritingLanguage"
import { useCreateProject } from "@/api/queries/projects"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useErrorText } from "@/i18n/errors"
import { uiLanguageFor } from "@/locales/languages"

const WRITING_LANGUAGES = ["zh-CN", "en"] as const satisfies readonly WritingLanguage[]

export function CreateProjectDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t(($) => $.library.create.title)}</DialogTitle>
          <DialogDescription>{t(($) => $.library.create.description)}</DialogDescription>
        </DialogHeader>
        {/* Mounted only while open, so every opening starts with an empty form. */}
        {open && <CreateProjectForm onCreated={() => onOpenChange(false)} />}
      </DialogContent>
    </Dialog>
  )
}

function CreateProjectForm({ onCreated }: { onCreated: () => void }) {
  const { t, i18n } = useTranslation()
  const errorText = useErrorText()
  const create = useCreateProject()
  const [name, setName] = useState("")
  const [novelTitle, setNovelTitle] = useState("")
  // A new work is written in the interface language unless the user picks another.
  const [writingLanguage, setWritingLanguage] = useState<WritingLanguage>(() =>
    uiLanguageFor(i18n.resolvedLanguage ?? i18n.language),
  )

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    create.mutate(
      { name, novelTitle, writingLanguage },
      {
        onSuccess: (project) => {
          toast.success(t(($) => $.library.done.created, { name: project.name }))
          onCreated()
        },
      },
    )
  }

  const nameError = errorForField(create.error, "project_name")
  const titleError = errorForField(create.error, "novel_title")
  const formError = errorForForm(create.error, ["project_name", "novel_title"])
  const languageItems = WRITING_LANGUAGES.map((value) => ({
    value,
    label: t(($) => $.common.writingLanguage[value]),
  }))

  return (
    <form onSubmit={submit} className="flex flex-col gap-6">
      <FieldGroup>
        <Field data-invalid={nameError !== undefined}>
          <FieldLabel htmlFor="create-project-name">{t(($) => $.library.create.name)}</FieldLabel>
          <Input
            id="create-project-name"
            value={name}
            onChange={(event) => setName(event.target.value)}
            aria-invalid={nameError !== undefined}
            autoFocus
          />
          {nameError !== undefined && <FieldError>{errorText(nameError)}</FieldError>}
        </Field>
        <Field data-invalid={titleError !== undefined}>
          <FieldLabel htmlFor="create-project-title">
            {t(($) => $.library.create.novelTitle)}
          </FieldLabel>
          <Input
            id="create-project-title"
            value={novelTitle}
            onChange={(event) => setNovelTitle(event.target.value)}
            aria-invalid={titleError !== undefined}
          />
          {titleError !== undefined && <FieldError>{errorText(titleError)}</FieldError>}
        </Field>
        <Field>
          <FieldLabel htmlFor="create-project-language">
            {t(($) => $.library.create.writingLanguage)}
          </FieldLabel>
          <Select
            items={languageItems}
            value={writingLanguage}
            onValueChange={(value: WritingLanguage | null) => {
              if (value) setWritingLanguage(value)
            }}
          >
            <SelectTrigger id="create-project-language" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {languageItems.map((item) => (
                <SelectItem key={item.value} value={item.value}>
                  {item.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <FieldDescription>{t(($) => $.library.create.writingLanguageHint)}</FieldDescription>
        </Field>
      </FieldGroup>
      {formError !== undefined && <ErrorAlert error={formError} />}
      <DialogFooter>
        <Button type="submit" disabled={create.isPending}>
          {create.isPending ? t(($) => $.common.status.saving) : t(($) => $.library.create.submit)}
        </Button>
      </DialogFooter>
    </form>
  )
}
