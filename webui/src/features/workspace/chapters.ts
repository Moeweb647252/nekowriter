import { useCallback } from "react"
import { useTranslation } from "react-i18next"

import type { ChapterLabel } from "@/api/generated/ChapterLabel"
import type { ChapterView } from "@/api/generated/ChapterView"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"

/** Names a chapter by its kind, number and title. */
export function useChapterName(): (chapter: ChapterLabel) => string {
  const { t } = useTranslation()
  return useCallback(
    (chapter: ChapterLabel) =>
      t(($) => $.workspace.chapterName[chapter.kind], {
        number: chapter.number,
        title: chapter.title,
      }),
    [t],
  )
}

/** The chapter the workflow is at, if any. */
export function currentChapter(view: WorkspaceView): ChapterView | undefined {
  const stage = view.version.stage
  return stage.phase === "chapter"
    ? view.chapters.find((chapter) => chapter.id === stage.chapterId)
    : undefined
}

/** The main chapter after `chapter` in the plan, if there is one. */
export function nextMainChapter(
  view: WorkspaceView,
  chapter: ChapterView,
): ChapterView | undefined {
  return chapter.kind === "main"
    ? view.chapters.find((other) => other.kind === "main" && other.number === chapter.number + 1)
    : undefined
}
