import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"

/** Covers the page once the service has shut down; nothing can be saved until it restarts. */
export function ServiceStopped() {
  const { t } = useTranslation()
  return (
    <div
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="service-stopped-title"
      aria-describedby="service-stopped-body"
      className="fixed inset-0 z-50 flex items-center justify-center bg-background/95 p-6"
    >
      <div className="flex max-w-md flex-col items-center gap-4 text-center">
        <h2 id="service-stopped-title" className="font-heading text-xl font-semibold">
          {t(($) => $.common.connection.stoppedTitle)}
        </h2>
        <p id="service-stopped-body" className="text-muted-foreground">
          {t(($) => $.common.connection.stoppedBody)}
        </p>
        <Button onClick={() => window.location.reload()}>
          {t(($) => $.common.actions.reload)}
        </Button>
      </div>
    </div>
  )
}
