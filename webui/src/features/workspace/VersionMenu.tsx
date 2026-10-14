import { ChevronDownIcon } from "lucide-react"
import { type SubmitEvent, useState } from "react"
import { useTranslation } from "react-i18next"
import { useNavigate } from "react-router"
import { toast } from "sonner"

import { DeleteVersionsDialog } from "./DeleteVersionsDialog"
import { workspacePath } from "./stage"
import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { ProjectView } from "@/api/generated/ProjectView"
import type { VersionId } from "@/api/generated/VersionId"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useCreateVersion, useSelectVersion } from "@/api/queries/projects"
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Field, FieldError, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { useErrorText } from "@/i18n/errors"

/** Switches between the work's versions and creates new, empty ones (§14). */
export function VersionMenu({ view }: { view: WorkspaceView }) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const errorText = useErrorText()
  const selectVersion = useSelectVersion()
  const [creating, setCreating] = useState(false)
  const [deleting, setDeleting] = useState(false)

  const open = (versionId: VersionId) => {
    if (versionId === view.version.id) return
    void navigate(workspacePath(view.project.id, versionId))
    // Remembering the choice is only for display; opening the version never waits for it.
    selectVersion.mutate(
      { project: view.project, versionId },
      { onError: (error) => toast.error(errorText(error)) },
    )
  }

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger render={<Button variant="outline" size="sm" />}>
          {t(($) => $.workspace.header.version, { number: view.version.number })}
          <ChevronDownIcon />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="min-w-64">
          <DropdownMenuGroup>
            <DropdownMenuLabel>{t(($) => $.workspace.header.versions)}</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              value={view.version.id}
              onValueChange={(value: VersionId) => open(value)}
            >
              {view.versions.map((version) => (
                <DropdownMenuRadioItem key={version.id} value={version.id}>
                  {t(($) => $.workspace.header.versionItem, {
                    number: version.number,
                    title: version.novelTitle,
                  })}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          <DropdownMenuItem onClick={() => setCreating(true)}>
            {t(($) => $.workspace.header.newVersion)}
          </DropdownMenuItem>
          {view.versions.length > 1 && (
            <DropdownMenuItem onClick={() => setDeleting(true)}>
              {t(($) => $.workspace.header.deleteVersions)}
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      <DeleteVersionsDialog view={view} open={deleting} onOpenChange={setDeleting} />
      <Dialog open={creating} onOpenChange={setCreating}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t(($) => $.workspace.newVersion.title)}</DialogTitle>
            <DialogDescription>{t(($) => $.workspace.newVersion.description)}</DialogDescription>
          </DialogHeader>
          {creating && (
            <NewVersionForm project={view.project} onCreated={() => setCreating(false)} />
          )}
        </DialogContent>
      </Dialog>
    </>
  )
}

function NewVersionForm({ project, onCreated }: { project: ProjectView; onCreated: () => void }) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const errorText = useErrorText()
  const create = useCreateVersion()
  const [novelTitle, setNovelTitle] = useState(project.selectedVersion.novelTitle)

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    create.mutate(
      { project, novelTitle },
      {
        onSuccess: (updated) => {
          toast.success(
            t(($) => $.workspace.newVersion.done, { number: updated.selectedVersion.number }),
          )
          onCreated()
          void navigate(workspacePath(updated.id, updated.selectedVersion.id))
        },
      },
    )
  }

  const titleError = errorForField(create.error, "novel_title")
  const formError = errorForForm(create.error, ["novel_title"])

  return (
    <form onSubmit={submit} className="flex flex-col gap-6">
      <Field data-invalid={titleError !== undefined}>
        <FieldLabel htmlFor="new-version-title">
          {t(($) => $.workspace.newVersion.novelTitle)}
        </FieldLabel>
        <Input
          id="new-version-title"
          value={novelTitle}
          onChange={(event) => setNovelTitle(event.target.value)}
          aria-invalid={titleError !== undefined}
          autoFocus
        />
        {titleError !== undefined && <FieldError>{errorText(titleError)}</FieldError>}
      </Field>
      {formError !== undefined && <ErrorAlert error={formError} />}
      <DialogFooter>
        <Button type="submit" disabled={create.isPending}>
          {create.isPending
            ? t(($) => $.common.status.saving)
            : t(($) => $.workspace.newVersion.submit)}
        </Button>
      </DialogFooter>
    </form>
  )
}
