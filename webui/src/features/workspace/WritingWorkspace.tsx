import { useTranslation } from "react-i18next"

import { ContentsNav } from "./ContentsNav"
import { useSelectedDocument } from "./documents"
import { DocumentView } from "./DocumentView"
import { WorkspaceLayout } from "./WorkspaceLayout"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"

/**
 * A version whose preparation is approved: the chapters and the next one to write. The
 * preparation documents stay readable from the contents.
 */
export function WritingWorkspace({ view }: { view: WorkspaceView }) {
  const { t } = useTranslation()
  const { selected, select } = useSelectedDocument(null)
  const stage = view.version.stage
  const next =
    stage.phase === "chapter"
      ? view.chapters.find((chapter) => chapter.id === stage.chapterId)
      : undefined

  const center =
    selected === null ? (
      <section>
        <h2 className="mb-6 font-heading text-2xl font-semibold">
          {t(($) => $.workspace.nav.chapters)}
        </h2>
        <ol className="flex flex-col gap-4">
          {view.chapters.map((chapter) => (
            <li key={chapter.id}>
              <p className="font-medium">
                {t(($) => $.workspace.nav.chapter, {
                  number: chapter.number,
                  title: chapter.title,
                })}
              </p>
              <p className="text-muted-foreground">{chapter.goal}</p>
            </li>
          ))}
        </ol>
      </section>
    ) : (
      <DocumentView view={view} doc={selected} />
    )

  const dock = (
    <div className="flex flex-col gap-2">
      <p className="text-muted-foreground">{t(($) => $.workspace.preparation.done)}</p>
      {next !== undefined && (
        <>
          <p className="font-medium">
            {t(($) => $.workspace.chapter.next, { number: next.number, title: next.title })}
          </p>
          <p>{t(($) => $.workspace.chapter.goal, { goal: next.goal })}</p>
        </>
      )}
      <p className="text-sm text-muted-foreground">{t(($) => $.workspace.chapter.notYet)}</p>
    </div>
  )

  return (
    <WorkspaceLayout
      view={view}
      contents={
        <ContentsNav view={view} selected={selected} onSelect={select} previewing={false} />
      }
      center={center}
      dock={dock}
    />
  )
}
