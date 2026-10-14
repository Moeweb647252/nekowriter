import { useTranslation } from "react-i18next"

import type { EngineList } from "@/api/generated/EngineList"
import { useEngines } from "@/api/queries/engines"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Skeleton } from "@/components/ui/skeleton"

/** Which research and accumulation engines exist (§12.3); never a fake capability. */
export function EnginesCard() {
  const { t } = useTranslation()
  const engines = useEngines()
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t(($) => $.settings.engines.title)}</CardTitle>
        <CardDescription>{t(($) => $.settings.engines.description)}</CardDescription>
      </CardHeader>
      <CardContent>
        {engines.isPending && <Skeleton className="h-12" />}
        {engines.isError && (
          <ErrorAlert error={engines.error} onRetry={() => void engines.refetch()} />
        )}
        {engines.isSuccess && (
          <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
            <dt className="text-muted-foreground">{t(($) => $.settings.engines.research)}</dt>
            <dd>
              <Engines list={engines.data.research} />
            </dd>
            <dt className="text-muted-foreground">{t(($) => $.settings.engines.accumulation)}</dt>
            <dd>
              <Engines list={engines.data.accumulation} />
            </dd>
          </dl>
        )}
      </CardContent>
    </Card>
  )
}

function Engines({ list }: { list: EngineList }) {
  const { t } = useTranslation()
  if (list.engines.length === 0) return t(($) => $.settings.engines.none)
  return list.engines.map((engine) => engine.name).join(", ")
}
