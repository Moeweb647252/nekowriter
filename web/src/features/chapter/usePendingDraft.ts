/**
 * The draft pane's view of a held chapter write (PROJECT §8.5, D-141).
 *
 * Two places in the chapter pane need the same answer: the text (`ChapterText` renders the merged
 * document) and the dock (`ChapterPage` decides the call). Deriving it once, here, is what keeps
 * them from disagreeing — the dock cannot say "applies when you approve" while the pane shows the
 * confirmed draft, or the reverse.
 *
 * The draft it compares against is the **open document on screen** (`chapterDoc`), not
 * `chapter.content` from the last query response: those differ while the author is typing, and the
 * version that decides whether a staged write is still current is the one the pane is showing.
 */
import { useMemo } from "react"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import { pendingDraftView, type PendingDraftView } from "@/features/chapter/pendingWrite"
import { useWriterState, type WriterState } from "@/features/chapter/useWriterState"
import { useChapterDoc } from "@/stores/chapterDoc"

export type PendingDraft = {
  writer: WriterState
  /**
   * The held write against the draft on screen, with its standing. Present whenever the held call
   * carries `pending_content`; `standing` says whether it would still apply.
   */
  view: PendingDraftView | undefined
}

/**
 * The writer state plus the view of its held write.
 *
 * `view` is `undefined` in the cases that mean "there is nothing staged to show": no held call, a
 * hold whose tool is not a chapter write, or a call that carries no `pending_content` (a hold from
 * before D-139, or a tool that does not stage). A write the draft has outrun is **not** one of
 * them: `standing` marks it superseded so the dock can say the staged text will not be applied,
 * instead of offering an apply that cannot happen.
 */
export function usePendingDraft(chapter: ChapterDto | undefined): PendingDraft {
  const writer = useWriterState(chapter, undefined)
  const paragraphs = useChapterDoc((state) => state.paragraphs)
  const version = useChapterDoc((state) => state.version)
  const held = writer.held
  const view = useMemo(
    () => pendingDraftView(held, { version, paragraphs }),
    [held, version, paragraphs],
  )
  return { writer, view }
}
