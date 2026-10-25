/** Localized error toasts (spec/11 §4.1, PROJECT §8.13).
 *
 * The API sends a code; the UI localizes it and falls back to the server's message when the code
 * is unknown (spec/10 §1). */
import { useCallback } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import type { ApiError } from "@/api/client"
import { isApiError } from "@/api/hooks/errors"

/** Returns a function that reports any thrown error as a localized toast. */
export function useErrorToast() {
  const { t } = useTranslation()
  // A stable identity: callers pass this into memoized children (the chapter paragraphs).
  return useCallback(
    (error: unknown) => {
      if (isApiError(error)) {
        toast.error(t(`errors:${error.code}`, { defaultValue: error.message }))
        return
      }
      toast.error(t("common:error_body"))
    },
    [t],
  )
}

/**
 * Reports mutation `warnings` (codes, spec/10 §1) as localized toasts: the operation succeeded,
 * so this is a warning rather than an error — e.g. a chapter approves but canon stays `pending`
 * because no archivist model is configured.
 */
export function useWarningToasts() {
  const { t } = useTranslation()
  return useCallback(
    (warnings: string[]) => {
      for (const code of warnings) {
        toast.warning(t(`errors:${code}`, { defaultValue: code }))
      }
    },
    [t],
  )
}

/** Localizes one error code. */
export function errorMessage(t: (key: string, options?: unknown) => string, error: ApiError): string {
  return t(`errors:${error.code}`, { defaultValue: error.message })
}
