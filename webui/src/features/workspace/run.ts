import { useCallback } from "react"
import { useTranslation } from "react-i18next"

import type { RunHalt } from "@/api/generated/RunHalt"
import type { RunStatus } from "@/api/generated/RunStatus"
import { useModelProblemText, useProviderFailureText } from "@/i18n/errors"
import { useFormatters } from "@/i18n/format"
import { assertNever } from "@/lib/assert"

/** Whether the run holds its version: working, waiting for the user, or interrupted. */
export function isActive(status: RunStatus): boolean {
  return status !== "stopped" && status !== "failed" && status !== "succeeded"
}

/** Whether the run ended its execution with a problem the user decides about. */
export function isHalted(status: RunStatus): boolean {
  return status === "interrupted" || status === "stopped" || status === "failed"
}

/** Says why a run stopped, failed or was interrupted, and what to do about it. */
export function useHaltText(): (halt: RunHalt) => string {
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
