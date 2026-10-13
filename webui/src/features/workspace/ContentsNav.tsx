import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"

import { useChapterName } from "./chapters"
import type { DocumentKey } from "./documents"
import type { ChapterId } from "@/api/generated/ChapterId"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { cn } from "@/lib/utils"

/** The workspace contents: the preparation documents, then the chapters (§18.2). */
export function ContentsNav({
  view,
  selected,
  onSelect,
  previewing,
  onSelectChapter,
}: {
  view: WorkspaceView
  selected: DocumentKey | null
  onSelect: (doc: DocumentKey | null) => void
  /** Whether a live preview can be shown. */
  previewing: boolean
  /** Shows the chapter in the reader. */
  onSelectChapter?: (chapter: ChapterId) => void
}) {
  const { t } = useTranslation()
  const chapterName = useChapterName()

  return (
    <nav aria-label={t(($) => $.workspace.nav.label)} className="flex flex-col gap-6 p-3 text-sm">
      <div className="flex flex-col gap-1">
        <h2 className="px-2 text-xs font-medium text-muted-foreground">
          {t(($) => $.workspace.nav.preparation)}
        </h2>
        <ul className="flex flex-col">
          {previewing && (
            <NavItem active={selected === null} onClick={() => onSelect(null)}>
              {t(($) => $.workspace.nav.preview)}
            </NavItem>
          )}
          <NavItem active={selected === "brief"} onClick={() => onSelect("brief")}>
            {t(($) => $.workspace.nav.brief)}
          </NavItem>
          {view.artifacts.map(({ kind, review }) => (
            <NavItem
              key={kind}
              active={selected === kind}
              onClick={() => onSelect(kind)}
              note={review === null ? undefined : t(($) => $.workspace.reviewStatus[review.status])}
            >
              {t(($) => $.workspace.artifact[kind])}
            </NavItem>
          ))}
        </ul>
      </div>
      <div className="flex flex-col gap-1">
        <h2 className="px-2 text-xs font-medium text-muted-foreground">
          {t(($) => $.workspace.nav.chapters)}
        </h2>
        {view.chapters.length === 0 ? (
          <p className="px-2 text-muted-foreground">{t(($) => $.workspace.nav.noChapters)}</p>
        ) : (
          <ol className="flex flex-col">
            {view.chapters.map((chapter) => (
              <NavItem
                key={chapter.id}
                active={false}
                onClick={() => onSelectChapter?.(chapter.id)}
                note={t(($) => $.workspace.chapterStatus[chapter.status])}
              >
                {chapterName(chapter)}
              </NavItem>
            ))}
          </ol>
        )}
      </div>
    </nav>
  )
}

function NavItem({
  active,
  onClick,
  note,
  children,
}: {
  active: boolean
  onClick: () => void
  note?: string | undefined
  children: ReactNode
}) {
  return (
    <li>
      <button
        type="button"
        aria-current={active ? "true" : undefined}
        onClick={onClick}
        className={cn(
          "flex w-full items-baseline justify-between gap-2 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-muted",
          active && "bg-muted font-medium",
        )}
      >
        <span className="truncate">{children}</span>
        {note !== undefined && (
          <span className="shrink-0 text-xs font-normal text-muted-foreground">{note}</span>
        )}
      </button>
    </li>
  )
}
