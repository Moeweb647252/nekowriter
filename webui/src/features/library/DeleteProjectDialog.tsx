import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import type { ProjectView } from "@/api/generated/ProjectView"
import { useDeleteProject } from "@/api/queries/projects"
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
} from "@/components/ui/alert-dialog"

export function DeleteProjectDialog({
  project,
  open,
  onOpenChange,
}: {
  project: ProjectView
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const remove = useDeleteProject()

  const confirm = () => {
    remove.mutate(project, {
      onSuccess: (deleted) => {
        toast.success(t(($) => $.library.done.deleted, { name: deleted.name }))
        onOpenChange(false)
      },
    })
  }

  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) remove.reset()
        onOpenChange(next)
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t(($) => $.library.delete.title, { name: project.name })}
          </AlertDialogTitle>
          <AlertDialogDescription>{t(($) => $.library.delete.body)}</AlertDialogDescription>
        </AlertDialogHeader>
        {remove.isError && <ErrorAlert error={remove.error} />}
        <AlertDialogFooter>
          <AlertDialogCancel>{t(($) => $.common.actions.cancel)}</AlertDialogCancel>
          <AlertDialogAction variant="destructive" disabled={remove.isPending} onClick={confirm}>
            {t(($) => $.library.delete.confirm)}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
