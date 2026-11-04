/** Localized timeline refusals (PROJECT §8.8, spec/10 §4, D-170).
 *
 * The server's `timeline.has_children` and `timeline.status_conflict` carry details the generic
 * `errors:` toast cannot render; turn them into one readable sentence per branch.
 */
import type { TFunction } from "i18next"

import { errorDetailNumber, errorDetails, isApiError } from "@/api/hooks/errors"

export function timelineErrorText(t: TFunction, error: unknown): string | null {
  if (!isApiError(error)) {
    return null
  }
  if (error.code === "timeline.has_children") {
    const count = errorDetailNumber(error, "children_count")
    return count === null
      ? t("errors:timeline.has_children")
      : t("canon:timeline_error.has_children", { count })
  }
  if (error.code === "timeline.status_conflict") {
    const details = errorDetails(error)
    const reason = typeof details?.reason === "string" ? details.reason : "backward"
    const conflict = typeof details?.conflict === "string" ? details.conflict : ""
    return t(`canon:timeline_error.${reason}`, { conflict })
  }
  return null
}
