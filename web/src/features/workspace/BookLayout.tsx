/** The book workspace: three resizable panels with persisted sizes (PROJECT §8.4, spec/11 §5.1). */
import { useEffect } from "react"
import { Outlet, useLocation, useParams } from "react-router"
import { useTranslation } from "react-i18next"
import { useQueryClient } from "@tanstack/react-query"
import { useDefaultLayout, usePanelRef } from "react-resizable-panels"

import { useBook } from "@/api/hooks/books"
import { connectEvents, disconnectEvents } from "@/api/events"
import { keys } from "@/api/keys"
import { BookBreadcrumb } from "@/features/workspace/BookBreadcrumb"
import { Navigator } from "@/features/workspace/Navigator"
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable"
import { Skeleton } from "@/components/ui/skeleton"
import { Button } from "@/components/ui/button"
import { PanelLeft } from "lucide-react"

/** The persisted layout id (spec/11 §5.1). */
const LAYOUT_ID = "book-workspace"

export default function BookLayout() {
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const location = useLocation()
  const book = useBook(bookId)

  const { defaultLayout, onLayoutChanged } = useDefaultLayout({
    id: LAYOUT_ID,
    storage: localStorage,
  })
  const navigatorRef = usePanelRef()

  // One SSE stream per open book (spec/11 §4.2).
  useEffect(() => {
    connectEvents(bookId, queryClient)
    return () => {
      disconnectEvents()
    }
  }, [bookId, queryClient])

  // The chapter page owns the open chapter document; leaving a chapter resets it.
  useEffect(() => {
    if (!location.pathname.includes("/chapters/")) {
      void queryClient.invalidateQueries({ queryKey: keys.chapters.detail(0) })
    }
  }, [location.pathname, queryClient])

  if (book.isPending) {
    return <Skeleton className="m-6 h-64" />
  }
  if (book.isError || book.data === undefined) {
    return (
      <div className="p-6">
        <p className="text-sm text-muted-foreground">{t("common:error_body")}</p>
        <Button variant="outline" className="mt-3" onClick={() => void book.refetch()}>
          {t("common:retry")}
        </Button>
      </div>
    )
  }

  return (
    <div className="flex h-[calc(100svh-3rem)] flex-col">
      <BookBreadcrumb book={book.data} />
      <ResizablePanelGroup
        id={LAYOUT_ID}
        defaultLayout={defaultLayout}
        onLayoutChanged={onLayoutChanged}
        className="min-h-0 flex-1"
      >
        <ResizablePanel
          id="nav"
          panelRef={navigatorRef}
          defaultSize="240px"
          minSize="200px"
          collapsible
          className="min-h-0"
        >
          <Navigator book={book.data} />
        </ResizablePanel>
        <ResizableHandle withHandle />
        <ResizablePanel id="main" minSize="320px" className="min-h-0 overflow-y-auto">
          <div className="relative min-h-full">
            <Button
              variant="ghost"
              size="icon"
              className="absolute top-2 left-2 z-10 hidden [@media(max-width:1023px)]:inline-flex"
              aria-label={t("book:sidebar_toggle")}
              onClick={() => navigatorRef.current?.expand()}
            >
              <PanelLeft />
            </Button>
            <Outlet context={{ book: book.data }} />
          </div>
        </ResizablePanel>
        <ResizableHandle withHandle />
        <ResizablePanel
          id="agent"
          defaultSize="380px"
          minSize="320px"
          collapsible
          className="min-h-0"
        >
          <AgentPanelPlaceholder />
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  )
}

/** The agent panel arrives in M3; until then the third panel says what it will hold. */
function AgentPanelPlaceholder() {
  const { t } = useTranslation()
  return (
    <div className="flex h-full flex-col border-l bg-sidebar/40">
      <div className="flex h-9 items-center border-b px-3 text-xs font-medium tracking-wide text-muted-foreground uppercase">
        {t("agent:panel_title")}
      </div>
      <div className="flex flex-1 items-center justify-center p-6 text-center text-xs text-muted-foreground">
        {t("agent:empty")}
      </div>
    </div>
  )
}
