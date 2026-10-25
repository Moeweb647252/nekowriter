/** Settings shell (PROJECT §8.12, spec/11 §3): providers, models, agent roles and appearance. */
import { Link, Outlet, useLocation, useParams } from "react-router"
import { useTranslation } from "react-i18next"
import { Cpu, Database, Globe, Languages, Server, UsersRound } from "lucide-react"

import { cn } from "@/lib/utils"

/** One settings section of the sidebar. */
type Section = {
  key: string
  icon: typeof Server
  label: string
}

export default function SettingsPage() {
  const { t } = useTranslation()
  const params = useParams()
  const location = useLocation()
  const sections: Section[] = [
    { key: "providers", icon: Server, label: t("settings:providers") },
    { key: "models", icon: Cpu, label: t("settings:models") },
    { key: "roles", icon: UsersRound, label: t("settings:roles") },
    { key: "appearance", icon: Languages, label: t("settings:appearance") },
    { key: "embeddings", icon: Database, label: t("settings:embeddings") },
    { key: "search", icon: Globe, label: t("settings:search") },
  ]
  const current = params.section ?? "providers"

  return (
    <div className="mx-auto flex w-full max-w-6xl gap-6 p-6">
      <aside className="w-44 shrink-0">
        <h1 className="mb-3 px-2 text-sm font-semibold">{t("settings:title")}</h1>
        <nav className="space-y-0.5">
          {sections.map((section) => {
            const Icon = section.icon
            return (
              <Link
                key={section.key}
                to={`/settings/${section.key}`}
                className={cn(
                  "flex items-center gap-2 rounded-md px-2 py-1.5 text-[13px] text-muted-foreground hover:bg-accent hover:text-foreground",
                  current === section.key && "bg-accent font-medium text-foreground",
                )}
              >
                <Icon className="size-3.5" aria-hidden="true" />
                {section.label}
              </Link>
            )
          })}
        </nav>
      </aside>
      <section className="min-w-0 flex-1" data-path={location.pathname}>
        <Outlet />
      </section>
    </div>
  )
}
