/** The chapter view (PROJECT §8.5, spec/11 §5.2). */
import { useEffect, useState } from "react"
import { useParams, useSearchParams } from "react-router"
import { useQueryClient } from "@tanstack/react-query"
import { useTranslation } from "react-i18next"

import { useBook } from "@/api/hooks/books"
import { useChapter, usePatchChapter } from "@/api/hooks/chapters"
import { ChapterHeader } from "@/features/chapter/ChapterHeader"
import { ChapterText } from "@/features/chapter/ChapterText"
import { OutlineTab } from "@/features/chapter/OutlineTab"
import { HistoryTab } from "@/features/chapter/HistoryTab"
import { EmptyState } from "@/components/common/EmptyState"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { keys } from "@/api/keys"
import { useChapterDoc } from "@/stores/chapterDoc"
import { isUnspaced } from "@/lib/language"

const TABS = ["text", "outline", "history"] as const
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

  const rawTab = searchParams.get("tab")
  const tab: Tab = TABS.includes(rawTab as Tab) ? (rawTab as Tab) : "text"
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
    <div className="flex min-h-full flex-col">
      <ChapterHeader
        chapter={document}
        settings={book.data.settings}
        language={book.data.writing_language}
        onRename={(title) => patchChapter.mutate({ title })}
        onWriterStarted={() => {
          // The agent panel picks the chapter's open session from `open_sessions` (spec/10 §2.8).
          void queryClient.invalidateQueries({ queryKey: keys.chapters.detail(chapterId) })
        }}
      />
      <Tabs
        value={tab}
        onValueChange={(next) => {
          const params = new URLSearchParams(searchParams)
          params.set("tab", next)
          setSearchParams(params, { replace: true })
        }}
        className="min-h-0 flex-1"
      >
        <div className="border-b px-6">
          <TabsList variant="line">
            <TabsTrigger value="text">{t("chapter:tab_text")}</TabsTrigger>
            <TabsTrigger value="outline">{t("chapter:tab_outline")}</TabsTrigger>
            <TabsTrigger value="history">{t("chapter:tab_history")}</TabsTrigger>
          </TabsList>
        </div>
        <TabsContent value="text" className="min-h-0 p-6">
          <ChapterText
            chapter={document}
            bookId={bookId}
            unspaced={unspaced}
            fullTextOpen={fullText}
            onFullTextOpenChange={setFullText}
          />
        </TabsContent>
        <TabsContent value="outline" className="min-h-0 p-6">
          <OutlineTab chapter={document} bookId={bookId} unspaced={unspaced} />
        </TabsContent>
        <TabsContent value="history" className="min-h-0 p-6">
          <HistoryTab chapter={document} bookId={bookId} unspaced={unspaced} />
        </TabsContent>
      </Tabs>
    </div>
  )
}
