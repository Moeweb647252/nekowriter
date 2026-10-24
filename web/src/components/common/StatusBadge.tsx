/** The status badge (spec/11 §7): every chapter, canon, session and research status maps to a
 * token, a Lucide icon and an i18n label. */
import {
  AlertTriangle,
  Ban,
  CheckCircle2,
  Circle,
  CircleDot,
  CircleSlash,
  Clock,
  Loader2,
  Pause,
  PenLine,
  RotateCcw,
  Sparkles,
  XCircle,
  type LucideIcon,
} from "lucide-react"
import { useTranslation } from "react-i18next"

import { cn } from "@/lib/utils"

type Token = "running" | "waiting" | "done" | "failed" | "idle"

type StatusSpec = {
  token: Token
  icon: LucideIcon
  /** i18n key inside the `chapter` namespace. */
  labelKey: string
  spin?: boolean
}

/** Status specs keyed by the wire value shared by chapters, canon and research (spec/02 §3). */
const STATUS: Record<string, StatusSpec> = {
  planned: { token: "idle", icon: Circle, labelKey: "status.planned" },
  drafting: { token: "running", icon: PenLine, labelKey: "status.drafting" },
  in_review: { token: "waiting", icon: Clock, labelKey: "status.in_review" },
  revising: { token: "running", icon: RotateCcw, labelKey: "status.revising" },
  official: { token: "done", icon: CheckCircle2, labelKey: "status.official" },
  none: { token: "idle", icon: CircleSlash, labelKey: "canon_status.none" },
  pending: { token: "waiting", icon: CircleDot, labelKey: "canon_status.pending" },
  syncing: { token: "running", icon: Loader2, labelKey: "canon_status.syncing", spin: true },
  synced: { token: "done", icon: CheckCircle2, labelKey: "canon_status.synced" },
  stale: { token: "waiting", icon: AlertTriangle, labelKey: "canon_status.stale" },
  failed: { token: "failed", icon: XCircle, labelKey: "canon_status.failed" },
  running_session: { token: "running", icon: Sparkles, labelKey: "status.revising" },
  held: { token: "waiting", icon: Pause, labelKey: "canon_status.pending" },
  completed: { token: "done", icon: CheckCircle2, labelKey: "status.official" },
  stopped: { token: "idle", icon: Ban, labelKey: "canon_status.none" },
  interrupted: { token: "idle", icon: Ban, labelKey: "canon_status.none" },
  abandoned: { token: "idle", icon: Ban, labelKey: "canon_status.none" },
  draft: { token: "idle", icon: Circle, labelKey: "status.planned" },
  ready: { token: "idle", icon: Circle, labelKey: "status.planned" },
  done: { token: "done", icon: CheckCircle2, labelKey: "status.official" },
  cancelled: { token: "idle", icon: Ban, labelKey: "canon_status.none" },
}

const TOKEN_CLASS: Record<Token, string> = {
  running: "text-status-running",
  waiting: "text-status-waiting",
  done: "text-status-done",
  failed: "text-status-failed",
  idle: "text-status-idle",
}

export type StatusBadgeProps = {
  status: string
  /** `chapter` renders the label from `chapter:status.*`; `canon` uses `chapter:canon_status.*`. */
  variant?: "chapter" | "canon" | "session"
  className?: string
  /** Icon only, with the label as the accessible name. */
  iconOnly?: boolean
}

/** Renders one status as an icon plus a localized label. */
export function StatusBadge({ status, variant = "chapter", className, iconOnly }: StatusBadgeProps) {
  const { t } = useTranslation()
  const spec =
    STATUS[status] ??
    (variant === "canon"
      ? STATUS.none
      : { token: "idle" as Token, icon: Circle, labelKey: "status.planned" })
  const Icon = spec.icon
  const label = t(`chapter:${spec.labelKey}`, { defaultValue: status })
  return (
    <span
      className={cn("inline-flex items-center gap-1 text-xs whitespace-nowrap", TOKEN_CLASS[spec.token], className)}
      title={label}
    >
      <Icon
        aria-hidden="true"
        className={cn("size-3.5 shrink-0", spec.spin && "animate-spin")}
      />
      {iconOnly ? <span className="sr-only">{label}</span> : <span>{label}</span>}
    </span>
  )
}
