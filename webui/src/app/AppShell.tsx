import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { NavLink, Outlet, useLocation } from "react-router"

import { ConnectionNotice } from "./ConnectionNotice"
import { cn } from "@/lib/utils"

/** The frame of the top-level pages: navigation, connection state and the page itself. */
export function AppShell() {
  const { t } = useTranslation()
  const nav = useRef<HTMLElement>(null)
  const { pathname } = useLocation()

  // On phones the row of links can hide the current page's link; bring it into view.
  useEffect(() => {
    nav.current
      ?.querySelector('[aria-current="page"]')
      ?.scrollIntoView({ block: "nearest", inline: "nearest" })
  }, [pathname])

  const navItems = [
    { to: "/works", label: t(($) => $.common.nav.works) },
    { to: "/dashboard", label: t(($) => $.common.nav.dashboard) },
    { to: "/prompts", label: t(($) => $.common.nav.prompts) },
    { to: "/knowledge", label: t(($) => $.common.nav.knowledge) },
    { to: "/settings", label: t(($) => $.common.nav.settings) },
  ]

  return (
    <div className="flex min-h-svh flex-col bg-background text-foreground">
      <header className="border-b">
        <div className="mx-auto flex max-w-6xl flex-col px-4 sm:h-14 sm:flex-row sm:items-center sm:gap-6">
          <span className="flex h-12 shrink-0 items-center font-heading text-base font-semibold sm:h-auto">
            {t(($) => $.common.appName)}
          </span>
          {/* On phones the links get a row of their own, which scrolls sideways instead of the page. */}
          <nav
            ref={nav}
            aria-label={t(($) => $.common.nav.label)}
            className="-mx-4 flex scroll-px-4 gap-1 overflow-x-auto px-4 pb-2 sm:mx-0 sm:overflow-visible sm:p-0"
          >
            {navItems.map((item) => (
              <NavLink
                key={item.to}
                to={item.to}
                className={({ isActive }) =>
                  cn(
                    "shrink-0 rounded-md px-3 py-1.5 text-sm whitespace-nowrap text-muted-foreground transition-colors hover:text-foreground",
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
