import { useTranslation } from "react-i18next"
import { NavLink, Outlet } from "react-router"

import { ConnectionNotice } from "./ConnectionNotice"
import { cn } from "@/lib/utils"

/** The frame of the top-level pages: navigation, connection state and the page itself. */
export function AppShell() {
  const { t } = useTranslation()

  const navItems = [
    { to: "/works", label: t(($) => $.common.nav.works) },
    { to: "/dashboard", label: t(($) => $.common.nav.dashboard) },
    { to: "/prompts", label: t(($) => $.common.nav.prompts) },
    { to: "/settings", label: t(($) => $.common.nav.settings) },
  ]

  return (
    <div className="flex min-h-svh flex-col bg-background text-foreground">
      <header className="border-b">
        <div className="mx-auto flex h-14 max-w-6xl items-center gap-6 px-4">
          <span className="font-heading text-base font-semibold">{t(($) => $.common.appName)}</span>
          <nav aria-label={t(($) => $.common.nav.label)} className="flex gap-1">
            {navItems.map((item) => (
              <NavLink
                key={item.to}
                to={item.to}
                className={({ isActive }) =>
                  cn(
                    "rounded-md px-3 py-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground",
                    isActive && "bg-muted text-foreground",
                  )
                }
              >
                {item.label}
              </NavLink>
            ))}
          </nav>
        </div>
      </header>
      <ConnectionNotice />
      <main className="mx-auto w-full max-w-6xl flex-1 px-4 py-8">
        <Outlet />
      </main>
    </div>
  )
}
