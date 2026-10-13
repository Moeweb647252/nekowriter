import { useCallback, useState } from "react"

import { currentChapter } from "../chapters"
import { ContentsNav } from "../ContentsNav"
import { useSelectedDocument } from "../documents"
import { DocumentView } from "../DocumentView"
import { WorkspaceLayout } from "../WorkspaceLayout"
import { ChapterDock, CompletedDock } from "./ChapterDock"
import { Reader } from "./Reader"
import { usePreview } from "@/api/events/preview"
import type { ChapterId } from "@/api/generated/ChapterId"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"

/**
 * A version whose preparation is approved: the reader of all chapters, and the dock for
 * the chapter being written. The preparation documents stay readable from the contents.
 */
export function WritingWorkspace({ view }: { view: WorkspaceView }) {
  const { selected, select } = useSelectedDocument(null)
  const [scrollTarget, setScrollTarget] = useState<ChapterId | null>(null)
  const preview = usePreview(view.run?.id, view.run?.requestInFlight)
  const stage = view.version.stage
  const chapter = currentChapter(view)

  const showChapter = useCallback(
    (id: ChapterId) => {
      select(null)
      setScrollTarget(id)
    },
    [select],
  )
  const scrolled = useCallback(() => setScrollTarget(null), [])

  const center =
    selected === null ? (
      <Reader view={view} preview={preview} scrollTarget={scrollTarget} onScrolled={scrolled} />
    ) : (
      <DocumentView view={view} doc={selected} />
    )
  const dock =
    stage.phase === "chapter" && chapter !== undefined ? (
      <ChapterDock view={view} chapter={chapter} step={stage.step} preview={preview} />
    ) : (
      <CompletedDock view={view} />
    )

  return (
    <WorkspaceLayout
      view={view}
      contents={
        <ContentsNav
          view={view}
          selected={selected}
          onSelect={select}
          previewing={false}
          onSelectChapter={showChapter}
        />
      }
      center={center}
      dock={dock}
    />
  )
}
