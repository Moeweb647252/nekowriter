import { Trash2Icon } from "lucide-react"
import { useTranslation } from "react-i18next"

import { useChapterName } from "../chapters"
import type { ChapterView } from "@/api/generated/ChapterView"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useDeleteChapter } from "@/api/queries/workspace"
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

/** Deletes an extra chapter, after confirmation (§14). */
export function DeleteChapterButton({
  view,
  chapter,
}: {
  view: WorkspaceView
  chapter: ChapterView
}) {
  const { t } = useTranslation()
  const chapterName = useChapterName()
  const remove = useDeleteChapter(view.version.id)
  return (
    <AlertDialog onOpenChange={(open) => open && remove.reset()}>
      <AlertDialogTrigger
        render={
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t(($) => $.workspace.deleteChapter.action)}
            title={t(($) => $.workspace.deleteChapter.action)}
          />
        }
      >
        <Trash2Icon />
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t(($) => $.workspace.deleteChapter.title, { chapter: chapterName(chapter) })}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t(($) => $.workspace.deleteChapter.body)}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {remove.isError && <ErrorAlert error={remove.error} />}
        <AlertDialogFooter>
          <AlertDialogCancel>{t(($) => $.common.actions.cancel)}</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={remove.isPending}
            onClick={() => remove.mutate({ revision: view.revision, chapterId: chapter.id })}
          >
            {t(($) => $.workspace.deleteChapter.confirm)}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
