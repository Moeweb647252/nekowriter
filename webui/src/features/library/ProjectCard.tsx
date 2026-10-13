import { EllipsisIcon } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Link } from "react-router"
import { toast } from "sonner"

import { DeleteProjectDialog } from "./DeleteProjectDialog"
import { RenameProjectDialog } from "./RenameProjectDialog"
import type { ProjectView } from "@/api/generated/ProjectView"
import { useRestoreProject } from "@/api/queries/projects"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { useErrorText } from "@/i18n/errors"
import { useStageLabel, workspacePath } from "@/features/workspace/stage"
import { useFormatters } from "@/i18n/format"

type Dialog = "rename" | "delete" | null

export function ProjectCard({ project }: { project: ProjectView }) {
  const { t } = useTranslation()
  const format = useFormatters()
  const stageLabel = useStageLabel()
  const [dialog, setDialog] = useState<Dialog>(null)
  const deleted = project.deletedAt !== null

  return (
    <Card className="h-full">
      <CardHeader>
        <CardTitle className="truncate">
          {deleted ? (
            project.name
          ) : (
            <Link
              to={workspacePath(project.id, project.selectedVersion.id)}
              className="hover:underline"
              aria-label={t(($) => $.library.card.open, { name: project.name })}
            >
              {project.name}
            </Link>
          )}
        </CardTitle>
        <CardDescription className="truncate">{project.selectedVersion.novelTitle}</CardDescription>
        {!deleted && (
          <CardAction>
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={t(($) => $.library.card.actions, { name: project.name })}
                  />
                }
              >
                <EllipsisIcon />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => setDialog("rename")}>
                  {t(($) => $.library.card.rename)}
                </DropdownMenuItem>
                <DropdownMenuItem variant="destructive" onClick={() => setDialog("delete")}>
                  {t(($) => $.library.card.delete)}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </CardAction>
        )}
      </CardHeader>
      <CardContent className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
        <Badge variant="secondary">
          {t(($) => $.common.writingLanguage[project.writingLanguage])}
        </Badge>
        <span>{t(($) => $.library.card.version, { number: project.selectedVersion.number })}</span>
        <span aria-hidden>·</span>
        <span>{stageLabel(project.selectedVersion.stage)}</span>
      </CardContent>
      <CardFooter className="mt-auto flex items-center justify-between gap-2 text-xs text-muted-foreground">
        {project.deletedAt === null ? (
          <span>
            {t(($) => $.library.card.updated, { time: format.dateTime(project.updatedAt) })}
          </span>
        ) : (
          <>
            <span>
              {t(($) => $.library.card.deleted, { time: format.dateTime(project.deletedAt) })}
            </span>
            <RestoreButton project={project} />
          </>
        )}
      </CardFooter>
      <RenameProjectDialog
        project={project}
        open={dialog === "rename"}
        onOpenChange={(open) => setDialog(open ? "rename" : null)}
      />
      <DeleteProjectDialog
        project={project}
        open={dialog === "delete"}
        onOpenChange={(open) => setDialog(open ? "delete" : null)}
      />
    </Card>
  )
}

function RestoreButton({ project }: { project: ProjectView }) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const restore = useRestoreProject()
  return (
    <Button
      variant="outline"
      size="sm"
      disabled={restore.isPending}
      onClick={() =>
        restore.mutate(project, {
          onSuccess: (restored) => {
            toast.success(t(($) => $.library.done.restored, { name: restored.name }))
          },
          onError: (error) => {
            toast.error(errorText(error))
          },
        })
      }
    >
      {t(($) => $.library.card.restore)}
    </Button>
  )
}
