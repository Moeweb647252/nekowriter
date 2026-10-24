/**
 * Session toasts (PROJECT §8.6, spec/11 §5.3): the author is told when a session needs attention.
 *
 * The panel compares the status of the open session between polls and raises one toast per
 * transition into a status that needs the author: held, awaiting input, failed, completed. The
 * comparison lives in [`statusNotification`] so it is unit-tested without React.
 */
import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import type { SessionDto } from "@/api/gen/SessionDto"

/** The kind of toast a status change deserves, or `null`. */
export type StatusNotification = "held" | "awaiting_input" | "failed" | "completed" | null

/**
 * The notification for a status transition (spec/11 §5.3). Only transitions into a status that
 * needs the author notify; a session that is already in that status does not re-notify.
 */
export function statusNotification(
  previous: SessionDto | undefined,
  current: SessionDto | undefined,
): StatusNotification {
  if (current === undefined) {
    return null
  }
  if (previous !== undefined && previous.id === current.id && previous.status === current.status) {
    return null
  }
  switch (current.status) {
    case "held":
      return "held"
    case "awaiting_input":
      return "awaiting_input"
    case "failed":
      return "failed"
    case "completed":
      return "completed"
    default:
      return null
  }
}

/** Raises the localized toast for a session status transition. */
export function useSessionToasts(session: SessionDto | undefined) {
  const { t } = useTranslation()
  const previous = useRef<SessionDto | undefined>(undefined)

  useEffect(() => {
    const notification = statusNotification(previous.current, session)
    previous.current = session
    if (notification === null || session === undefined) {
      return
    }
    const key = `agent:toast.${notification}`
    switch (notification) {
      case "held":
      case "awaiting_input":
        toast.message(t(key))
        break
      case "failed":
        toast.error(t(key))
        break
      case "completed":
        toast.success(t(key))
        break
    }
  }, [session, t])
}
