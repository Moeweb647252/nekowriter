import { useTranslation } from "react-i18next"

import { useHealth } from "@/api/queries/system"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Skeleton } from "@/components/ui/skeleton"

/** Where this installation keeps its data and logs, for support and backups. */
export function DiagnosticsCard() {
  const { t } = useTranslation()
  const health = useHealth()

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t(($) => $.settings.diagnostics.title)}</CardTitle>
      </CardHeader>
      <CardContent>
        {health.isPending && <Skeleton className="h-20" />}
        {health.isError && (
          <ErrorAlert error={health.error} onRetry={() => void health.refetch()} />
        )}
        {health.isSuccess && (
          <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
            <dt className="text-muted-foreground">{t(($) => $.settings.diagnostics.version)}</dt>
            <dd>{health.data.appVersion}</dd>
            <dt className="text-muted-foreground">{t(($) => $.settings.diagnostics.dataDir)}</dt>
            <dd className="font-mono break-all">{health.data.dataDir}</dd>
            <dt className="text-muted-foreground">{t(($) => $.settings.diagnostics.logDir)}</dt>
            <dd className="font-mono break-all">{health.data.logDir}</dd>
          </dl>
        )}
      </CardContent>
    </Card>
  )
}
