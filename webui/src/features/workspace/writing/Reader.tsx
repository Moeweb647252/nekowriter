import { ArrowDownIcon } from "lucide-react"
import { useContext, useEffect, useLayoutEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"

import { currentChapter, useChapterName } from "../chapters"
import { ReadingText } from "../DocumentView"
import { ScrollContainerContext } from "../scroll"
import { citable, proposalTarget, scalarLength, wholeRevision } from "../citations"
import { normalizeLineEndings } from "./edits"
import type { ChapterId } from "@/api/generated/ChapterId"
import type { ChapterView } from "@/api/generated/ChapterView"
import type { Preview } from "@/api/generated/Preview"
import type { ProposalView } from "@/api/generated/ProposalView"
import type { RevisionId } from "@/api/generated/RevisionId"
import type { RevisionView } from "@/api/generated/RevisionView"
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
          held={chapter.id === current?.id ? heldAppend(view) : null}
          proposal={chapter.id === current?.id ? view.proposal : null}
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

/** The text a paused run holds back from appending, if it holds an append. */
function heldAppend(view: WorkspaceView): string | null {
  const edit = view.held?.edit
  return edit?.op === "append" ? edit.content : null
}

/**
 * The body with a proposed replacement marked in place: the removed text struck through
 * and the added text after it, each with a symbol, a color and a label (§18.4). Everything
 * but the removed text is citable as the text the proposal would produce.
 */
function ProposedReplace({
  body,
  proposal,
  edit,
}: {
  body: RevisionView
  proposal: ProposalView
  edit: { oldStr: string; newStr: string }
}) {
  const { t } = useTranslation()
  const text = body.content
  const oldStr = normalizeLineEndings(edit.oldStr)
  const newStr = normalizeLineEndings(edit.newStr)
  const at = text.indexOf(oldStr)
  if (at < 0) return <ReadingText text={text} cite={wholeRevision(body.id)} />
  const target = proposalTarget(proposal)
  const before = text.slice(0, at)
  return (
    <>
      <p className="mb-2 text-xs text-muted-foreground">{t(($) => $.workspace.proposal.label)}</p>
      <div className="font-reading text-lg leading-[1.85] whitespace-pre-wrap">
        <span {...citable({ target, at: 0 })}>{before}</span>
        <del className="rounded-sm bg-destructive/15 text-destructive before:content-['-']">
          <span className="sr-only">{t(($) => $.workspace.proposal.removed)}: </span>
          {oldStr}
        </del>
        {newStr !== "" && (
          <ins className="rounded-sm bg-emerald-500/15 text-emerald-900 no-underline before:content-['+'] dark:text-emerald-200">
            <span className="sr-only">{t(($) => $.workspace.proposal.added)}: </span>
            <span {...citable({ target, at: scalarLength(before) })}>{newStr}</span>
          </ins>
        )}
        <span {...citable({ target, at: scalarLength(before) + scalarLength(newStr) })}>
          {text.slice(at + oldStr.length)}
        </span>
      </div>
    </>
  )
}

function sectionId(chapter: ChapterId): string {
  return `chapter-${chapter}`
}

function ChapterSection({
  chapter,
  streaming,
  held,
  proposal,
}: {
  chapter: ChapterView
  streaming: StreamingEdit | null
  /** Text a paused run holds back. */
  held: string | null
  /** An edit waiting for the user's review. */
  proposal: ProposalView | null
}) {
  const { t } = useTranslation()
  const chapterName = useChapterName()
  const body = useRevision(chapter.body?.id, { keepPrevious: true })
  const headingId = `${sectionId(chapter.id)}-title`
  const appended = streaming?.appended ?? null
  const edit = proposal?.edit
  // The committed body; undefined while it loads.
  const base = chapter.body === null ? "" : body.data?.content
  // While an edit is proposed, the body is cited as part of the text it would produce.
  const bodyCite = (revision: RevisionId) =>
    proposal === null ? wholeRevision(revision) : { target: proposalTarget(proposal), at: 0 }

  return (
    <section id={sectionId(chapter.id)} aria-labelledby={headingId} className="scroll-mt-6">
      <h2 id={headingId} className="mb-6 font-heading text-2xl font-semibold">
        {chapterName(chapter)}
      </h2>
      {chapter.body === null ? (
        appended === null &&
        held === null && (
          <p className="text-muted-foreground">{t(($) => $.workspace.reader.empty)}</p>
        )
      ) : body.data === undefined ? (
        <Skeleton className="h-32 w-full" />
      ) : proposal !== null && edit?.op === "replace" ? (
        <ProposedReplace body={body.data} proposal={proposal} edit={edit} />
      ) : (
        <ReadingText text={body.data.content} cite={bodyCite(body.data.id)} />
      )}
      {proposal !== null && edit?.op === "append" && (
        <div className="mt-2 border-l-2 border-emerald-600/60 pl-4">
          <p className="mb-1 text-xs text-muted-foreground">
            + {t(($) => $.workspace.proposal.label)}
          </p>
          <ins className="block text-emerald-900 no-underline dark:text-emerald-200">
            <span className="sr-only">{t(($) => $.workspace.proposal.added)}: </span>
            <ReadingText
              text={normalizeLineEndings(edit.content)}
              cite={
                base === undefined
                  ? undefined
                  : { target: proposalTarget(proposal), at: scalarLength(base) }
              }
            />
          </ins>
        </div>
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
      {held !== null && (
        <div className="mt-2 border-l-2 border-dotted border-muted-foreground/60 pl-4">
          <p className="mb-1 text-xs text-muted-foreground">{t(($) => $.workspace.reader.held)}</p>
          <div className="text-foreground/60">
            <ReadingText text={held} />
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
