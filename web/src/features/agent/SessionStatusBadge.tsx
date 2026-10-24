/**
 * The status badge of a session (PROJECT §8.6). It reads the `SessionStatus` from the generated
 * types and colors it with the semantic tokens.
 */
import { useTranslation } from "react-i18next"

import type { SessionStatus } from "@/api/gen/SessionStatus"
import { Badge } from "@/components/ui/badge"
import { cn } from "@/lib/utils"

/** The badge tone of a status. */
function toneOf(status: SessionStatus): string {
  switch (status) {
    case "running":
      return "border-transparent bg-primary/15 text-primary"
    case "held":
    case "awaiting_input":
      return "border-transparent bg-amber-500/15 text-amber-700 dark:text-amber-400"
    case "paused":
      return "border-transparent bg-orange-500/15 text-orange-700 dark:text-orange-400"
    case "failed":
      return "border-transparent bg-destructive/15 text-destructive"
    case "stopped":
    case "interrupted":
      return "border-transparent bg-muted text-muted-foreground"
    case "completed":
      return "border-transparent bg-emerald-500/15 text-emerald-700 dark:text-emerald-400"
    case "idle":
      return "border-transparent bg-sky-500/15 text-sky-700 dark:text-sky-400"
    case "superseded":
    case "abandoned":
      return "border-transparent bg-muted text-muted-foreground"
    default:
      return ""
  }
}

export function SessionStatusBadge({
  status,
  pausing = false,
  stopping = false,
  className,
}: {
  status: SessionStatus
  /** The author pressed Pause; the badge says "Pausing…" until the status changes. */
  pausing?: boolean
  /** The author pressed Stop; the badge says "Stopping…". */
  stopping?: boolean
  className?: string
}) {
  const { t } = useTranslation()
  if (pausing) {
    return (
      <Badge variant="outline" className={cn("animate-pulse", className)}>
        {t("agent:status.pausing")}
      </Badge>
    )
  }
  if (stopping) {
    return (
      <Badge variant="outline" className={cn("animate-pulse", className)}>
        {t("agent:status.stopping")}
      </Badge>
    )
  }
  return (
    <Badge variant="outline" className={cn(toneOf(status), className)}>
      {t(`agent:status.${status}`)}
    </Badge>
  )
}
