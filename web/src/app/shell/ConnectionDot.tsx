import { useTranslation } from "react-i18next"

import { useLiveStore } from "@/stores/live"
import { cn } from "@/lib/utils"

/** The SSE connection-state dot (spec/11 §4.2). */
export function ConnectionDot() {
  const { t } = useTranslation()
  const connection = useLiveStore((state) => state.connection)
  const label =
    connection === "open"
      ? t("common:connection_open")
      : connection === "reconnecting"
        ? t("common:connection_reconnecting")
        : t("common:connection_connecting")
  return (
    <span className="flex items-center gap-1.5 text-xs text-muted-foreground" title={label}>
      <span
        aria-hidden="true"
        className={cn(
          "size-2 rounded-full",
          connection === "open" && "bg-status-done",
          connection === "reconnecting" && "bg-status-waiting",
          connection === "connecting" && "bg-status-idle",
        )}
      />
      <span className="sr-only" aria-live="polite">
        {label}
      </span>
    </span>
  )
}
