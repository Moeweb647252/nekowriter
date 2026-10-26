/** The book workspace: three resizable panels with persisted sizes (PROJECT §8.4, spec/11 §5.1). */
import { useCallback, useEffect, useMemo, useState } from "react"
import { Outlet, useLocation, useParams } from "react-router"
import { useTranslation } from "react-i18next"
import { useQueryClient } from "@tanstack/react-query"
import { useDefaultLayout, usePanelRef } from "react-resizable-panels"

import { useBook } from "@/api/hooks/books"
import { connectEvents, disconnectEvents } from "@/api/events"
import { keys } from "@/api/keys"
import { AgentPanel } from "@/features/agent/AgentPanel"
import { BookBreadcrumb } from "@/features/workspace/BookBreadcrumb"
import { Navigator } from "@/features/workspace/Navigator"
import type { BookShell } from "@/features/workspace/shell"
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable"
import { Skeleton } from "@/components/ui/skeleton"
import { Button } from "@/components/ui/button"
import { PanelLeft } from "lucide-react"

/** The persisted layout id (spec/11 §5.1). */
const LAYOUT_ID = "book-workspace"

/**
 * True for the conception studio route (PROJECT §8.7, spec/11 §5.4).
 *
 * The studio is the **only** workspace route whose content owns its height: it is a two-column
 * app shell whose conversation pane and document pane each scroll on their own, with a composer
 * that must never scroll away. Every other route (the chapter page, the overview, the canon and
 * settings-materials pages) is a long document that relies on this layout's main panel scrolling
 * the whole page (`ChapterPage` starts with `flex min-h-full flex-col`). The route decides, so the
 * fixed-height mode can never reach those routes.
 */
export function isConceptionStudio(pathname: string): boolean {
  return pathname.endsWith("/conception")
}

export default function BookLayout() {
  const { bookId: rawBookId, chapterId: rawChapterId } = useParams()
  const bookId = Number(rawBookId)
  // The agent panel opens the chapter's own session when a chapter is on screen (spec/11 §5.3).
  const chapterId = rawChapterId === undefined ? undefined : Number(rawChapterId)
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const location = useLocation()
  const book = useBook(bookId)
  // The conception studio is full width: the agent panel is collapsed by default there, because
  // the studio's own conversation pane is the agent panel (PROJECT §8.7, spec/11 §5.4).
  const isConception = isConceptionStudio(location.pathname)

  const { defaultLayout, onLayoutChanged } = useDefaultLayout({
    id: LAYOUT_ID,
    storage: localStorage,
  })
  const navigatorRef = usePanelRef()
  const agentRef = usePanelRef()
  // The agent panel is `collapsible` (spec/11 §5.1) and its size is remembered per browser, so an
  // author can leave it collapsed — and the page that starts a run watched there has to know
  // (D-135). The panel answers with `onResize` (0 px *is* collapsed for a collapsible panel), so
  // the hint cannot disagree with what is on screen; the store's `agentPanelCollapsed` field was
  // never written by anything and would drift. `expandAgentPanel` is the other direction.
  const [agentPanelCollapsed, setAgentPanelCollapsed] = useState(false)
  const expandAgentPanel = useCallback(() => {
    agentRef.current?.expand()
  }, [agentRef])

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

  // Memoized (above the early returns, so the hook order is stable) because routed pages put the
  // shell in callback dependencies. `null` only while the book is loading or failed, which is
  // exactly when no routed page renders.
  const shell = useMemo<BookShell | null>(
    () =>
      book.data === undefined
        ? null
        : {
            book: book.data,
            // The studio renders no agent panel, so nothing there is collapsed (spec/11 §5.4).
            agentPanelCollapsed: isConception ? false : agentPanelCollapsed,
            expandAgentPanel,
          },
    [book.data, isConception, agentPanelCollapsed, expandAgentPanel],
  )

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
        {/*
          Route-aware height chain (PROJECT §8.7, spec/11 §5.4). react-resizable-panels always
          renders this wrapper as a scroll container (`overflow: auto` inline), so the difference
          is what the child is told:
          - every other route is a page: the wrapper keeps `min-h-full` (content height) and the
            panel scrolls the whole page, which is what `ChapterPage`'s `flex min-h-full flex-col`
            and the canon/settings-materials pages are built for;
          - the studio owns its height: the wrapper is exactly the panel's height (`h-full`,
            `overflow: hidden`) so `ConceptionStudio`'s `flex h-full min-h-0 flex-col` resolves and
            its conversation pane and document pane become the only two scroll containers.
        */}
        <ResizablePanel
          id="main"
          minSize="320px"
          className={isConception ? "min-h-0" : "min-h-0 overflow-y-auto"}
          // react-resizable-panels sets `overflow: auto` inline on this element, so only an inline
          // style can take the studio's outer scrollbar away. Other routes keep it.
          style={isConception ? { overflow: "hidden" } : undefined}
        >
          <div className={isConception ? "relative h-full" : "relative min-h-full"}>
            <Button
              variant="ghost"
              size="icon"
              className="absolute top-2 left-2 z-10 hidden [@media(max-width:1023px)]:inline-flex"
              aria-label={t("book:sidebar_toggle")}
              onClick={() => navigatorRef.current?.expand()}
            >
              <PanelLeft />
            </Button>
            <Outlet context={shell} />
          </div>
        </ResizablePanel>
        {isConception ? null : (
          <>
            <ResizableHandle withHandle />
            <ResizablePanel
              id="agent"
              panelRef={agentRef}
              defaultSize="380px"
              minSize="320px"
              collapsible
              className="min-h-0"
              // 0 px is a collapsible panel's collapsed size (react-resizable-panels defaults
              // `collapsedSize` to 0), so this is the panel reporting its own state.
              onResize={(size) => setAgentPanelCollapsed(size.inPixels === 0)}
            >
              <AgentPanel bookId={bookId} chapterId={chapterId} />
            </ResizablePanel>
          </>
        )}
      </ResizablePanelGroup>
    </div>
  )
}
