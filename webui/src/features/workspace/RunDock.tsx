import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"

import { useHaltText } from "./run"
import type { RunView } from "@/api/generated/RunView"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useContinueRun, useStopRun } from "@/api/queries/workspace"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"

function RequestCount({ run }: { run: RunView }) {
  const { t } = useTranslation()
  return (
    <p className="text-xs text-muted-foreground">
      {t(($) => $.workspace.run.requests, {
        count: run.requestCount,
        limit: run.requestAllowanceEnd,
      })}
    </p>
  )
}

/** A run that is waiting for the model or writing, which the user can stop (§16.3). */
export function RunProgress({
  view,
  run,
  children,
}: {
  view: WorkspaceView
  run: RunView
  children: ReactNode
}) {
  const { t } = useTranslation()
  const stop = useStopRun(view.version.id)
  return (
    <div className="flex flex-col items-start gap-2">
      <div role="status" className="flex flex-col gap-2">
        <p className="flex items-center gap-2 font-medium">
          <Spinner />
          {t(($) => $.workspace.run.status[run.status])}
        </p>
        <p className="text-muted-foreground">{children}</p>
        <RequestCount run={run} />
      </div>
      {stop.isError && <ErrorAlert error={stop.error} />}
      <Button
        variant="outline"
        size="sm"
        disabled={stop.isPending}
        onClick={() => stop.mutate(run.id)}
      >
        {t(($) => $.workspace.run.stop)}
      </Button>
    </div>
  )
}

/** A run that stopped, failed or was interrupted (§18.3 execution problem). */
export function RunProblem({ view, run }: { view: WorkspaceView; run: RunView }) {
  const { t } = useTranslation()
  const haltText = useHaltText()
  const continueRun = useContinueRun(view.version.id)

  return (
    <div className="flex flex-col items-start gap-3">
      <div className="flex flex-col gap-1">
        <p className="font-medium">{t(($) => $.workspace.run.status[run.status])}</p>
        {run.halt !== null && <p>{haltText(run.halt)}</p>}
        <RequestCount run={run} />
      </div>
      {continueRun.isError && <ErrorAlert error={continueRun.error} />}
      <Button
        disabled={continueRun.isPending}
        onClick={() => continueRun.mutate({ revision: view.revision, runId: run.id })}
      >
        {t(($) => $.workspace.run.continue)}
      </Button>
    </div>
  )
}
