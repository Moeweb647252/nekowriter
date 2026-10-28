import { useState } from "react"
import { Link, NavLink, Outlet } from "react-router"
import { useTranslation } from "react-i18next"
import { Search } from "lucide-react"

import { ConnectionDot } from "@/app/shell/ConnectionDot"
import { LanguageSwitch } from "@/app/shell/LanguageSwitch"
import { QuickSwitch } from "@/app/shell/QuickSwitch"
import { ThemeToggle } from "@/app/shell/ThemeToggle"
import { Button } from "@/components/ui/button"
import { useHotkey } from "@/lib/hotkeys"

/** The application chrome: a top bar over the routed page (spec/11 §5.1). */
export function AppShell() {
  const { t } = useTranslation()
  const [quickSwitchOpen, setQuickSwitchOpen] = useState(false)
  // ⌘/Ctrl+K opens the quick switch (PROJECT §8.13, spec/11 §7). The hook ignores keystrokes that
  // belong to an IME composition, so a Chinese input method never triggers it.
  useHotkey({ key: "k", primary: true }, (event) => {
    event.preventDefault()
    setQuickSwitchOpen((open) => !open)
  })
  const navClass = ({ isActive }: { isActive: boolean }) =>
    isActive
      ? "rounded-md px-2 py-1 text-[13px] font-medium text-foreground"
      : "rounded-md px-2 py-1 text-[13px] text-muted-foreground hover:bg-accent hover:text-foreground"
  return (
    <div className="flex min-h-svh flex-col bg-background">
      <header className="flex h-12 shrink-0 items-center justify-between border-b px-4">
        <div className="flex items-center gap-3">
          <Link to="/" className="font-semibold tracking-tight">
            NekoWriter
          </Link>
          <nav className="flex items-center gap-1" aria-label={t("common:app_name")}>
            <NavLink to="/" end className={navClass}>
              {t("common:library")}
            </NavLink>
            <NavLink to="/knowledge" className={navClass}>
              {t("common:knowledge")}
            </NavLink>
            <NavLink to="/skills" className={navClass}>
              {t("common:skills")}
            </NavLink>
            <NavLink to="/prompts" className={navClass}>
              {t("common:prompts")}
            </NavLink>
            <NavLink to="/settings" className={navClass}>
              {t("common:settings")}
            </NavLink>
          </nav>
          <ConnectionDot />
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            className="gap-2 text-xs text-muted-foreground"
            aria-label={t("common:quick_switch")}
            onClick={() => setQuickSwitchOpen(true)}
          >
            <Search aria-hidden="true" />
            <span className="hidden sm:inline">{t("common:quick_switch")}</span>
            <kbd className="hidden rounded border px-1 font-mono text-[10px] sm:inline">⌘K</kbd>
          </Button>
          <LanguageSwitch />
          <ThemeToggle />
        </div>
      </header>
      <main className="min-h-0 flex-1">
        <Outlet />
      </main>
      <QuickSwitch open={quickSwitchOpen} onOpenChange={setQuickSwitchOpen} />
    </div>
  )
}
