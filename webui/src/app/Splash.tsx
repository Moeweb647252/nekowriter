import { useTranslation } from "react-i18next"

import { Spinner } from "@/components/ui/spinner"

/** Shown while the app connects to the service for the first time. */
export function Splash() {
  const { t } = useTranslation()
  return (
    <div className="flex min-h-svh items-center justify-center gap-3 text-muted-foreground">
      <Spinner />
      <span>{t(($) => $.common.status.loading)}</span>
    </div>
  )
}
