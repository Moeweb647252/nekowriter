import { type ReactNode, useState } from "react"
import { useTranslation } from "react-i18next"

import { WorkspaceHeader } from "./WorkspaceHeader"
import { ConnectionNotice } from "@/app/ConnectionNotice"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet"

/**
 * The workspace frame (§18.2): the header, the contents on the left, the reader in the
 * center and the dock with the current step at the bottom. Below 1100px the contents
 * move into a drawer.
 */
export function WorkspaceLayout({
  view,
  contents,
  center,
  dock,
}: {
  view: WorkspaceView
  contents: ReactNode
  center: ReactNode
  dock: ReactNode
}) {
  const { t } = useTranslation()
  const [contentsOpen, setContentsOpen] = useState(false)

  return (
    <div className="flex h-svh flex-col bg-background text-foreground">
      <WorkspaceHeader view={view} onShowContents={() => setContentsOpen(true)} />
      <ConnectionNotice />
      <div className="flex min-h-0 flex-1">
        <aside className="hidden w-60 shrink-0 overflow-y-auto border-r min-[1100px]:block">
          {contents}
        </aside>
        <main className="min-w-0 flex-1 overflow-y-auto">
          <div className="mx-auto w-full max-w-[52rem] px-6 py-10">{center}</div>
        </main>
      </div>
      <section
        aria-label={t(($) => $.workspace.dock)}
        className="max-h-[45svh] min-h-48 shrink-0 overflow-y-auto border-t bg-muted/40"
      >
        <div className="mx-auto w-full max-w-[52rem] px-6 py-4">{dock}</div>
      </section>
      <Sheet open={contentsOpen} onOpenChange={setContentsOpen}>
        <SheetContent side="left" className="w-72">
          <SheetHeader>
            <SheetTitle>{t(($) => $.workspace.nav.label)}</SheetTitle>
          </SheetHeader>
          {/* Choosing an entry closes the drawer. */}
          <div onClickCapture={() => setContentsOpen(false)}>{contents}</div>
        </SheetContent>
      </Sheet>
    </div>
  )
}
