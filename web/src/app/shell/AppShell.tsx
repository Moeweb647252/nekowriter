import { Link, NavLink, Outlet } from "react-router"
import { useTranslation } from "react-i18next"

import { ConnectionDot } from "@/app/shell/ConnectionDot"
import { LanguageSwitch } from "@/app/shell/LanguageSwitch"
import { ThemeToggle } from "@/app/shell/ThemeToggle"

/** The application chrome: a top bar over the routed page (spec/11 §5.1). */
export function AppShell() {
  const { t } = useTranslation()
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
