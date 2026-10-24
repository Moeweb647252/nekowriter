/** A full-width inline error (a settings page cannot rely on a toast for every failure). */
import { useTranslation } from "react-i18next"
import { AlertCircle } from "lucide-react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"

export type ErrorToastInlineProps = {
  /** Localized detail; defaults to the generic error body. */
  message?: string
}

export function ErrorToastInline({ message }: ErrorToastInlineProps) {
  const { t } = useTranslation()
  return (
    <Alert variant="destructive">
      <AlertCircle className="size-4" />
      <AlertTitle>{t("common:error_title")}</AlertTitle>
      <AlertDescription>{message ?? t("common:error_body")}</AlertDescription>
    </Alert>
  )
}
