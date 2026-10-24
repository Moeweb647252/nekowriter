import { Outlet } from "react-router"

import { ConnectionDot } from "@/app/shell/ConnectionDot"
import { LanguageSwitch } from "@/app/shell/LanguageSwitch"
import { ThemeToggle } from "@/app/shell/ThemeToggle"

/** The application chrome: a top bar over the routed page (spec/11 §5.1). */
export function AppShell() {
  return (
    <div className="flex min-h-svh flex-col bg-background">
      <header className="flex h-12 shrink-0 items-center justify-between border-b px-4">
        <div className="flex items-center gap-3">
          <span className="font-semibold tracking-tight">NekoWriter</span>
          <ConnectionDot />
        </div>
        <div className="flex items-center gap-2">
          <LanguageSwitch />
          <ThemeToggle />
        </div>
      </header>
      <main className="min-h-0 flex-1">
        <Outlet />
      </main>
    </div>
  )
}
