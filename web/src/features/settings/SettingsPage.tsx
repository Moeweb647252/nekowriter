/** Settings shell (PROJECT §8.12, spec/11 §3): providers, models, agent roles and appearance. */
import { Link, Outlet, useLocation, useParams } from "react-router"
import { useTranslation } from "react-i18next"
import { Cpu, Database, Globe, Languages, Server, UsersRound } from "lucide-react"

import { cn } from "@/lib/utils"

/**
 * The sidebar sections, in order. The route table must have one child route per key (a test in
 * `app/router.test.ts` enforces it): a nav item without a route renders an empty settings page.
 */
export const SETTINGS_SECTION_KEYS = [
  "providers",
  "models",
  "roles",
  "appearance",
  "embeddings",
  "search",
] as const

type SectionKey = (typeof SETTINGS_SECTION_KEYS)[number]

/** One settings section of the sidebar. */
type Section = {
  key: SectionKey
  icon: typeof Server
  label: string
}

const SECTION_ICONS: Record<SectionKey, typeof Server> = {
  providers: Server,
  models: Cpu,
  roles: UsersRound,
  appearance: Languages,
  embeddings: Database,
  search: Globe,
}

const SECTION_LABELS: Record<SectionKey, string> = {
  providers: "settings:providers",
  models: "settings:models",
  roles: "settings:roles",
  appearance: "settings:appearance",
  embeddings: "settings:embeddings",
  search: "settings:search",
}

export default function SettingsPage() {
  const { t } = useTranslation()
  const params = useParams()
  const location = useLocation()
  const sections: Section[] = SETTINGS_SECTION_KEYS.map((key) => ({
    key,
    icon: SECTION_ICONS[key],
    label: t(SECTION_LABELS[key]),
  }))
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
