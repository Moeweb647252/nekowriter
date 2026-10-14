import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import type { VersionId } from "@/api/generated/VersionId"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useDeleteVersion } from "@/api/queries/projects"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"

/**
 * Deleting the work's other versions (§14). Each deletion is confirmed in its row; the
 * version shown now is not offered.
 */
export function DeleteVersionsDialog({
  view,
  open,
  onOpenChange,
}: {
  view: WorkspaceView
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const remove = useDeleteVersion()
  const [confirming, setConfirming] = useState<VersionId | null>(null)
  const others = view.versions.filter(
    (version) => version.id !== view.project.selectedVersion.id && version.id !== view.version.id,
  )

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        onOpenChange(next)
        setConfirming(null)
        remove.reset()
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t(($) => $.workspace.deleteVersions.title)}</DialogTitle>
          <DialogDescription>{t(($) => $.workspace.deleteVersions.description)}</DialogDescription>
        </DialogHeader>
        {others.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t(($) => $.workspace.deleteVersions.none)}
          </p>
        ) : (
          <ul className="flex flex-col gap-2">
            {others.map((version) => (
              <li
                key={version.id}
                className="flex flex-wrap items-center justify-between gap-2 rounded-md border px-3 py-2 text-sm"
              >
                <span>
                  {t(($) => $.workspace.header.versionItem, {
                    number: version.number,
                    title: version.novelTitle,
                  })}
                </span>
                {confirming === version.id ? (
                  <span className="flex gap-2">
                    <Button
                      variant="destructive"
                      size="sm"
                      disabled={remove.isPending}
                      onClick={() =>
                        remove.mutate(
                          { project: view.project, versionId: version.id },
                          {
                            onSuccess: () => {
                              setConfirming(null)
                              toast.success(
                                t(($) => $.workspace.deleteVersions.done, {
                                  number: version.number,
                                }),
                              )
                            },
                          },
                        )
                      }
                    >
                      {t(($) => $.workspace.deleteVersions.confirm, { number: version.number })}
                    </Button>
                    <Button variant="ghost" size="sm" onClick={() => setConfirming(null)}>
                      {t(($) => $.workspace.deleteVersions.cancel)}
                    </Button>
                  </span>
                ) : (
                  <Button variant="outline" size="sm" onClick={() => setConfirming(version.id)}>
                    {t(($) => $.workspace.deleteVersions.delete)}
                  </Button>
                )}
              </li>
            ))}
          </ul>
        )}
        {remove.isError && <ErrorAlert error={remove.error} />}
      </DialogContent>
    </Dialog>
  )
}
