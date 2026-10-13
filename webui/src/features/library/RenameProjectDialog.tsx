import { type SubmitEvent, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { ProjectView } from "@/api/generated/ProjectView"
import { useRenameProject } from "@/api/queries/projects"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Field, FieldError, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { useErrorText } from "@/i18n/errors"

export function RenameProjectDialog({
  project,
  open,
  onOpenChange,
}: {
  project: ProjectView
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t(($) => $.library.rename.title)}</DialogTitle>
        </DialogHeader>
        {open && <RenameProjectForm project={project} onRenamed={() => onOpenChange(false)} />}
      </DialogContent>
    </Dialog>
  )
}

function RenameProjectForm({
  project,
  onRenamed,
}: {
  project: ProjectView
  onRenamed: () => void
}) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const rename = useRenameProject()
  const [name, setName] = useState<string>(project.name)

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    rename.mutate(
      { project, name },
      {
        onSuccess: (renamed) => {
          toast.success(t(($) => $.library.done.renamed, { name: renamed.name }))
          onRenamed()
        },
      },
    )
  }

  const nameError = errorForField(rename.error, "project_name")
  const formError = errorForForm(rename.error, ["project_name"])

  return (
    <form onSubmit={submit} className="flex flex-col gap-6">
      <Field data-invalid={nameError !== undefined}>
        <FieldLabel htmlFor="rename-project-name">{t(($) => $.library.rename.name)}</FieldLabel>
        <Input
          id="rename-project-name"
          value={name}
          onChange={(event) => setName(event.target.value)}
          aria-invalid={nameError !== undefined}
          autoFocus
        />
        {nameError !== undefined && <FieldError>{errorText(nameError)}</FieldError>}
      </Field>
      {formError !== undefined && <ErrorAlert error={formError} />}
      <DialogFooter>
        <Button type="submit" disabled={rename.isPending}>
          {rename.isPending ? t(($) => $.common.status.saving) : t(($) => $.library.rename.submit)}
        </Button>
      </DialogFooter>
    </form>
  )
}
