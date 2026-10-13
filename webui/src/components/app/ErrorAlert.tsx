import { CircleAlertIcon } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Alert, AlertAction, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { useErrorText } from "@/i18n/errors"

/** Describes a failed request, with an optional way to try again. */
export function ErrorAlert({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  return (
    <Alert variant="destructive">
      <CircleAlertIcon />
      <AlertDescription>{errorText(error)}</AlertDescription>
      {onRetry && (
        <AlertAction>
          <Button variant="outline" size="sm" onClick={onRetry}>
            {t(($) => $.common.actions.retry)}
          </Button>
        </AlertAction>
      )}
    </Alert>
  )
}
