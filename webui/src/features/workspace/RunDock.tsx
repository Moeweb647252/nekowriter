import type { ReactNode } from "react"
import { useCallback } from "react"
import { useTranslation } from "react-i18next"

import type { RunHalt } from "@/api/generated/RunHalt"
import type { RunView } from "@/api/generated/RunView"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useContinueRun, useStopRun } from "@/api/queries/workspace"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { useModelProblemText, useProviderFailureText } from "@/i18n/errors"
import { useFormatters } from "@/i18n/format"
import { assertNever } from "@/lib/assert"

/** Says why a run stopped, failed or was interrupted, and what to do about it. */
function useHaltText(): (halt: RunHalt) => string {
  const { t } = useTranslation()
  const format = useFormatters()
  const modelProblem = useModelProblemText()
  const providerFailure = useProviderFailureText()
  return useCallback(
    (halt: RunHalt) => {
      switch (halt.kind) {
        case "model_not_ready":
          return t(($) => $.workspace.run.halt.model_not_ready, {
            problem: modelProblem(halt.problem),
          })
        case "context_limit_exceeded":
          return t(($) => $.workspace.run.halt.context_limit_exceeded, {
            estimated: format.number(halt.estimatedTokens),
            limit: format.number(halt.limitTokens),
          })
        case "provider":
          return t(($) => $.workspace.run.halt.provider, {
            failure: providerFailure(halt.failure),
          })
        case "request_limit":
        case "protocol_error_limit":
        case "process_stopped":
        case "stopped_by_user":
        case "internal":
          return t(($) => $.workspace.run.halt[halt.kind])
        default:
          return assertNever(halt)
      }
    },
    [t, format, modelProblem, providerFailure],
  )
}

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
