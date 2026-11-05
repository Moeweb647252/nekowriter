/**
 * The chapter view (PROJECT §8.5, spec/11 §5.2).
 *
 * The Text tab is where the body is written, so it is also where a held chapter write is decided:
 * the dock under the reading column (`WriterDock`) owns that decision, next to the merged text
 * it would produce (D-141). The agent panel deliberately offers no second decision surface for the
 * same call — the author reported the split (「正文编写要在中间的正文草稿区进行，正文编辑时的意见
 * 提出在草稿区下方的 dock 栏里进行」).
 *
 * The dock is rendered here rather than inside `ChapterText` because it must outlive the pane's
 * scroll: it is sticky at the bottom of this column, and this page is what owns the column. It also
 * needs the query client for the resume mutation, which is why it is not a leaf of the text view.
 */
import { useEffect, useState } from "react"
import { useParams, useSearchParams } from "react-router"
import { useQueryClient } from "@tanstack/react-query"
import { useTranslation } from "react-i18next"

import { useBook } from "@/api/hooks/books"
import { useChapter, usePatchChapter } from "@/api/hooks/chapters"
import { useSession } from "@/api/hooks/sessions"
import { ChapterHeader } from "@/features/chapter/ChapterHeader"
import { ChapterText } from "@/features/chapter/ChapterText"
import { openOutlineSession, outlineState } from "@/features/chapter/outlineState"
import { OutlineTab } from "@/features/chapter/OutlineTab"
import { usePendingDraft } from "@/features/chapter/usePendingDraft"
import { WriterDock } from "@/features/chapter/WriterDock"
import { ReviewTab } from "@/features/chapter/ReviewTab"
import { HistoryTab } from "@/features/chapter/HistoryTab"
import { EmptyState } from "@/components/common/EmptyState"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { keys } from "@/api/keys"
import { useChapterDoc } from "@/stores/chapterDoc"
import { useUiStore } from "@/stores/ui"
import { useBookShell } from "@/features/workspace/shell"
import { focusSessionInAgentPanel } from "@/features/chapter/focusSession"
import { isUnspaced } from "@/lib/language"

const TABS = ["text", "outline", "review", "history"] as const
type Tab = (typeof TABS)[number]

export default function ChapterPage() {
  const { bookId: rawBookId, chapterId: rawChapterId } = useParams()
  const bookId = Number(rawBookId)
  const chapterId = Number(rawChapterId)
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [searchParams, setSearchParams] = useSearchParams()
  const book = useBook(bookId)
  const chapter = useChapter(chapterId)
  const patchChapter = usePatchChapter(chapterId, bookId)
  const load = useChapterDoc((state) => state.load)
  const reset = useChapterDoc((state) => state.reset)
  const shell = useBookShell()
  const focusStartedSession = (sessionId: number) => {
    // A newly started session must win over an older open one in the panel (R3-08).
    useUiStore.getState().setPinnedSession(bookId, sessionId)
    focusSessionInAgentPanel(shell)
  }

  const rawTab = searchParams.get("tab")
  const tab: Tab = TABS.includes(rawTab as Tab) ? (rawTab as Tab) : "text"
  // A review issue jumps to the paragraph it quotes (`?tab=text&pid=p12`).
  const focusPid = searchParams.get("pid") ?? undefined
  const [fullText, setFullText] = useState(false)

  // The store owns the open chapter's paragraphs (spec/11 §4.3). It is reloaded only when the
  // chapter or its stored version changes: a `chapter.updated` event rewrites the chapter query
  // for its metadata (the version stays the same), and reloading then would replace the ops the
  // store just applied with stale cached content.
  const loadedId = chapter.data?.id
  const loadedVersion = chapter.data?.content_version
  const loadedContent = chapter.data?.content
  useEffect(() => {
    if (loadedId !== undefined && loadedVersion !== undefined && loadedContent !== undefined) {
      load(loadedId, loadedVersion, loadedContent)
    }
  }, [loadedId, loadedVersion, loadedContent, load])

  useEffect(
    () => () => {
      reset()
    },
    [reset],
  )

  // The held write and its merged view, derived once for both the pane (which renders it) and the
  // dock (which decides it). Hooks stay above the early returns.
  const pending = usePendingDraft(chapter.data)
  // One derivation for the header and the Outline tab (D-146). The snapshot supplies the held
  // call, so a pause-held retrieval call is in progress rather than "awaiting approval".
  const outlineSession = openOutlineSession(chapter.data)
  const outlineSnapshot = useSession(outlineSession?.status === "held" ? outlineSession.id : null)
  const outlineStateValue = outlineState(chapter.data, outlineSnapshot.data?.tool_calls)

  if (chapter.isPending || book.isPending) {
    return <Skeleton className="m-6 h-64" />
  }
  if (chapter.isError || chapter.data === undefined || book.data === undefined) {
    return (
      <div className="p-6">
        <EmptyState title={t("common:error_title")} description={t("common:error_body")} />
      </div>
    )
  }

  const document = chapter.data
  const unspaced = isUnspaced(book.data.writing_language)

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ChapterHeader
        key={document.id}
        chapter={document}
        settings={book.data.settings}
        language={book.data.writing_language}
        // The header counts what the chapter holds; a staged write is a labelled projection beside
        // it (D-141), so the two panes never show one number for two different documents.
        projectedWords={
          pending.view?.standing === "current" ? pending.view.stagedWords : undefined
        }
        outlineState={outlineStateValue}
        onRename={(title) => patchChapter.mutate({ title })}
        onSessionStarted={(sessionId) => {
          focusStartedSession(sessionId)
          // The agent panel picks the chapter's open session from `open_sessions` (spec/10 §2.8).
          void queryClient.invalidateQueries({ queryKey: keys.chapters.detail(chapterId) })
          void queryClient.invalidateQueries({ queryKey: keys.chapters.reviews(chapterId) })
        }}
      />
      <Tabs
        value={tab}
        onValueChange={(next) => {
          const params = new URLSearchParams(searchParams)
          params.set("tab", next)
          setSearchParams(params, { replace: true })
        }}
        className="flex min-h-0 flex-1 flex-col"
      >
        <div className="border-b px-6">
          <TabsList variant="line">
            <TabsTrigger value="text">{t("chapter:tab_text")}</TabsTrigger>
            <TabsTrigger value="outline">{t("chapter:tab_outline")}</TabsTrigger>
            <TabsTrigger value="review">{t("chapter:tab_review")}</TabsTrigger>
            <TabsTrigger value="history">{t("chapter:tab_history")}</TabsTrigger>
          </TabsList>
        </div>
        <TabsContent value="text" className="flex min-h-0 flex-1 flex-col p-0">
          <div data-draft-region className="flex min-h-0 flex-1 flex-col">
          <div className="min-h-0 flex-1 overflow-auto p-6">
          <ChapterText
            key={document.id}
            chapter={document}
            bookId={bookId}
            unspaced={unspaced}
            fullTextOpen={fullText}
            onFullTextOpenChange={setFullText}
            focusPid={tab === "text" ? focusPid : undefined}
            // The merged view is rendered only when the staged write would still apply; a
            // superseded one keeps the confirmed draft on screen (see `pendingWrite.ts`).
            pendingView={pending.view?.standing === "current" ? pending.view : undefined}
          />
          </div>
          <WriterDock
            key={pending.writer.session?.id ?? "none"}
            bookId={bookId}
            chapterId={chapterId}
            session={pending.writer.session}
            held={pending.writer.held}
            view={pending.view}
          />
          </div>
        </TabsContent>
        <TabsContent value="outline" className="min-h-0 p-6">
          <OutlineTab
            key={document.id}
            chapter={document}
            bookId={bookId}
            unspaced={unspaced}
            settings={book.data.settings}
            state={outlineStateValue}
            session={outlineSession}
            calls={outlineSnapshot.data?.tool_calls}
            onSessionStarted={(sessionId) => {
              focusStartedSession(sessionId)
              void queryClient.invalidateQueries({ queryKey: keys.chapters.detail(chapterId) })
            }}
            onFocusSession={(sessionId) => {
              useUiStore.getState().setPinnedSession(bookId, sessionId)
              focusSessionInAgentPanel(shell)
              void queryClient.invalidateQueries({ queryKey: keys.sessions.detail(sessionId) })
            }}
          />
        </TabsContent>
        <TabsContent value="review" className="min-h-0 p-6">
          <ReviewTab
            key={document.id}
            chapter={document}
            bookId={bookId}
            settings={book.data.settings}
            onSessionStarted={(sessionId) => {
              focusStartedSession(sessionId)
              void queryClient.invalidateQueries({ queryKey: keys.chapters.detail(chapterId) })
              void queryClient.invalidateQueries({ queryKey: keys.chapters.reviews(chapterId) })
            }}
            onFocusPid={(pid) => {
              const params = new URLSearchParams(searchParams)
              params.set("tab", "text")
              params.set("pid", pid)
              setSearchParams(params, { replace: true })
            }}
          />
        </TabsContent>
        <TabsContent value="history" className="min-h-0 p-6">
          <HistoryTab chapter={document} bookId={bookId} unspaced={unspaced} />
        </TabsContent>
      </Tabs>
    </div>
  )
}
