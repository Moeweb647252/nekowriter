import { CircleCheckIcon, TriangleAlertIcon } from "lucide-react"
import { useTranslation } from "react-i18next"

import type { TestModelRequest } from "@/api/generated/TestModelRequest"
import { useModelTest } from "@/api/queries/settings"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { useFormatters } from "@/i18n/format"

/**
 * Tests a model with one real request and shows the outcome (§13.3). `request` builds the
 * request from the form as it is now.
 */
export function ModelTest({
  request,
  disabled = false,
}: {
  request: () => TestModelRequest
  disabled?: boolean
}) {
  const { t } = useTranslation()
  const format = useFormatters()
  const test = useModelTest()

  return (
    <div className="flex flex-col items-start gap-3">
      <Button
        type="button"
        variant="outline"
        disabled={disabled || test.isPending}
        onClick={() => test.mutate(request())}
      >
        {test.isPending ? t(($) => $.settings.test.running) : t(($) => $.settings.test.run)}
      </Button>
      {test.isError && <ErrorAlert error={test.error} />}
      {test.isSuccess && (
        <Alert>
          <CircleCheckIcon />
          <AlertDescription>
            {t(($) => $.settings.test.passed, {
              seconds: format.number(Math.round(test.data.elapsedMillis / 100) / 10),
            })}
          </AlertDescription>
        </Alert>
      )}
      {test.isSuccess && test.variables.model.reportsStreamUsage && !test.data.usageReported && (
        <Alert>
          <TriangleAlertIcon />
          <AlertDescription>{t(($) => $.settings.test.usageMissing)}</AlertDescription>
        </Alert>
      )}
    </div>
  )
}
