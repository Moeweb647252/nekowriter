import { useTranslation } from "react-i18next"

import { ServiceStopped } from "./ServiceStopped"
import { useConnectionStatus } from "@/api/events/connection"

/** Says when the live connection to the service is being restored or the service stopped. */
export function ConnectionNotice() {
  const { t } = useTranslation()
  const connection = useConnectionStatus()
  return (
    <>
      {connection === "reconnecting" && (
        <div role="status" className="bg-muted px-4 py-2 text-center text-sm text-muted-foreground">
          {t(($) => $.common.connection.reconnecting)}
        </div>
      )}
      {connection === "stopped" && <ServiceStopped />}
    </>
  )
}
