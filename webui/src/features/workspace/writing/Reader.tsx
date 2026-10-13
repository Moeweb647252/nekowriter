import { ArrowDownIcon } from "lucide-react"
import { useContext, useEffect, useLayoutEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"

import { currentChapter, useChapterName } from "../chapters"
import { ReadingText } from "../DocumentView"
import { ScrollContainerContext } from "../scroll"
import type { ChapterId } from "@/api/generated/ChapterId"
import type { ChapterView } from "@/api/generated/ChapterView"
import type { Preview } from "@/api/generated/Preview"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useRevision } from "@/api/queries/workspace"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { readPartialObject, textOf } from "@/lib/partialJson"

/** What the agent is doing to the body while its answer streams. */
interface StreamingEdit {
  /** Text being appended, not committed yet. */
  appended: string | null
  /** Whether a replacement is being prepared. */
  editing: boolean
}

function streamingEdit(preview: Preview | null): StreamingEdit {
  const call = preview?.toolCalls.at(-1)
  switch (call?.name) {
    case "append_content":
      return {
        appended: textOf(readPartialObject(call.arguments).content) ?? "",
        editing: false,
      }
    case "replace_content":
      return { appended: null, editing: true }
    default:
      return { appended: null, editing: false }
  }
}

/**
 * The continuous reader (§18.2, §18.4): every chapter's committed body in order, with the
 * text being written shown apart from it. While the reader is at the end and nothing is
 * selected, it follows new text; otherwise it keeps its place and offers a way down.
 */
export function Reader({
  view,
  preview,
  scrollTarget,
  onScrolled,
}: {
  view: WorkspaceView
  preview: Preview | null
  /** A chapter to bring into view. */
  scrollTarget: ChapterId | null
  onScrolled: () => void
}) {
  const { t } = useTranslation()
  const container = useContext(ScrollContainerContext)
  const end = useRef<HTMLDivElement>(null)
  const atEnd = useRef(false)
  const [endVisible, setEndVisible] = useState(true)
  const current = currentChapter(view)
  const streaming = streamingEdit(preview)

  useEffect(() => {
    const element = end.current
    if (element === null) return
    const observer = new IntersectionObserver(
      ([entry]) => {
        const visible = entry?.isIntersecting ?? false
        atEnd.current = visible
        setEndVisible(visible)
      },
      { root: container.current },
    )
    observer.observe(element)
    return () => observer.disconnect()
  }, [container])

  // Opening the workspace shows the chapter being worked on.
  const opened = useRef(false)
  useEffect(() => {
    if (opened.current || current === undefined) return
    opened.current = true
    document.getElementById(sectionId(current.id))?.scrollIntoView({ block: "start" })
  }, [current])

  useEffect(() => {
    if (scrollTarget === null) return
    document.getElementById(sectionId(scrollTarget))?.scrollIntoView({ block: "start" })
    onScrolled()
  }, [scrollTarget, onScrolled])

  const growth = `${current?.body?.id ?? ""}:${String(streaming.appended?.length ?? 0)}`
  useLayoutEffect(() => {
    const selecting = !(document.getSelection()?.isCollapsed ?? true)
    if (atEnd.current && !selecting) end.current?.scrollIntoView({ block: "end" })
  }, [growth])

  const writing = streaming.appended !== null || streaming.editing
  return (
    <article aria-label={t(($) => $.workspace.reader.label)} className="flex flex-col gap-16">
      {view.chapters.map((chapter) => (
        <ChapterSection
          key={chapter.id}
          chapter={chapter}
          streaming={chapter.id === current?.id ? streaming : null}
        />
      ))}
      <div ref={end} aria-hidden className="h-px" />
      {writing && !endVisible && (
        <div className="sticky bottom-4 flex justify-center">
          <Button
            variant="secondary"
            size="sm"
            onClick={() => end.current?.scrollIntoView({ block: "end", behavior: "smooth" })}
          >
            <ArrowDownIcon />
            {t(($) => $.workspace.reader.newContent)}
          </Button>
        </div>
      )}
    </article>
  )
}

function sectionId(chapter: ChapterId): string {
  return `chapter-${chapter}`
}

function ChapterSection({
  chapter,
  streaming,
}: {
  chapter: ChapterView
  streaming: StreamingEdit | null
}) {
  const { t } = useTranslation()
  const chapterName = useChapterName()
  const body = useRevision(chapter.body?.id, { keepPrevious: true })
  const headingId = `${sectionId(chapter.id)}-title`
  const appended = streaming?.appended ?? null

  return (
    <section id={sectionId(chapter.id)} aria-labelledby={headingId} className="scroll-mt-6">
      <h2 id={headingId} className="mb-6 font-heading text-2xl font-semibold">
        {chapterName(chapter)}
      </h2>
      {chapter.body === null ? (
        appended === null && (
          <p className="text-muted-foreground">{t(($) => $.workspace.reader.empty)}</p>
        )
      ) : body.data === undefined ? (
        <Skeleton className="h-32 w-full" />
      ) : (
        <ReadingText text={body.data.content} />
      )}
      {appended !== null && (
        <div className="mt-2 border-l-2 border-dashed border-primary/50 pl-4">
          <p className="mb-1 text-xs text-muted-foreground">
            {t(($) => $.workspace.reader.writing)}
          </p>
          <div className="text-foreground/75">
            <ReadingText text={appended} />
          </div>
        </div>
      )}
      {streaming?.editing === true && (
        <p role="status" className="mt-2 text-sm text-muted-foreground">
          {t(($) => $.workspace.reader.editing)}
        </p>
      )}
    </section>
  )
}
