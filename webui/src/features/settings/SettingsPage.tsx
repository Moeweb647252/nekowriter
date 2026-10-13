import { useTranslation } from "react-i18next"

import { AppearanceSettings } from "./AppearanceSettings"
import { DiagnosticsCard } from "./DiagnosticsCard"
import { ModelSettingsCard } from "./ModelSettingsCard"
import { ServiceCard } from "./ServiceCard"

export function SettingsPage() {
  const { t } = useTranslation()
  return (
    <div className="flex max-w-3xl flex-col gap-6">
      <h1 className="font-heading text-2xl font-semibold">{t(($) => $.settings.title)}</h1>
      <ModelSettingsCard />
      <AppearanceSettings />
      <DiagnosticsCard />
      <ServiceCard />
    </div>
  )
}
