/** Localized error toasts (spec/11 §4.1, PROJECT §8.13).
 *
 * The API sends a code; the UI localizes it and falls back to the server's message when the code
 * is unknown (spec/10 §1). */
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import type { ApiError } from "@/api/client"
import { isApiError } from "@/api/hooks/errors"

/** Returns a function that reports any thrown error as a localized toast. */
export function useErrorToast() {
  const { t } = useTranslation()
  return (error: unknown) => {
    if (isApiError(error)) {
      toast.error(t(`errors:${error.code}`, { defaultValue: error.message }))
      return
    }
    toast.error(t("common:error_body"))
  }
}

/** Localizes one error code. */
export function errorMessage(t: (key: string, options?: unknown) => string, error: ApiError): string {
  return t(`errors:${error.code}`, { defaultValue: error.message })
}
