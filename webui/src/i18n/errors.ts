import { useCallback, useMemo } from "react"
import { useTranslation } from "react-i18next"

import { ApiError, NetworkError } from "@/api/client"
import type { ApiErrorBody } from "@/api/generated/ApiErrorBody"
import type { AnswerProblem } from "@/api/generated/AnswerProblem"
import type { ApiProblem } from "@/api/generated/ApiProblem"
import type { CitationProblem } from "@/api/generated/CitationProblem"
import type { Field } from "@/api/generated/Field"
import type { FieldProblem } from "@/api/generated/FieldProblem"
import type { ModelProblem } from "@/api/generated/ModelProblem"
import type { ProviderFailure } from "@/api/generated/ProviderFailure"
import type { Resource } from "@/api/generated/Resource"
import type { StateConflict } from "@/api/generated/StateConflict"
import type { TimeoutPhase } from "@/api/generated/TimeoutPhase"
import { assertNever } from "@/lib/assert"

/**
 * The shape of the `errors` namespace: a message for every error code the API can
 * return, so adding a code on the server fails type checking until it is translated.
 */
export interface ErrorCatalog {
  network: string
  unexpected: string
  reference: string
  field: Record<Field, string>
  resource: Record<Resource, string>
  modelProblem: Record<ModelProblem["kind"], string>
  providerFailure: Record<ProviderFailure["kind"], string>
  timeoutPhase: Record<TimeoutPhase, string>
  codes: {
    [Code in ApiProblem["code"]]: Code extends "FIELD_INVALID"
      ? Record<FieldProblem["kind"], string>
      : Code extends "STATE_CONFLICT"
        ? Record<StateConflict, string>
        : Code extends "ANSWER_INVALID"
          ? Record<AnswerProblem["kind"], string>
          : Code extends "CITATION_INVALID"
            ? Record<CitationProblem["kind"], string>
            : string
  }
}

/** Describes why the model settings do not allow requests, and how to fix it. */
export function useModelProblemText(): (problem: ModelProblem) => string {
  const { t } = useTranslation()
  return useCallback((problem) => t(($) => $.errors.modelProblem[problem.kind]), [t])
}

/** Describes how a request to the model service failed, as a clause. */
export function useProviderFailureText(): (failure: ProviderFailure) => string {
  const { t } = useTranslation()
  return useCallback(
    (failure: ProviderFailure) => {
      switch (failure.kind) {
        case "timeout":
          return t(($) => $.errors.providerFailure.timeout, {
            phase: t(($) => $.errors.timeoutPhase[failure.phase]),
          })
        case "http_status":
          return t(($) => $.errors.providerFailure.http_status, { status: failure.status })
        case "connect":
        case "invalid_response":
        case "response_too_large":
          return t(($) => $.errors.providerFailure[failure.kind])
        default:
          return assertNever(failure)
      }
    },
    [t],
  )
}

/** Returns a function that describes any thrown error in the interface language. */
export function useErrorText(): (error: unknown) => string {
  const { t } = useTranslation()
  const modelProblem = useModelProblemText()
  const providerFailure = useProviderFailureText()

  const describeProblem = useMemo(() => {
    const fieldProblem = (field: string, problem: FieldProblem): string => {
      switch (problem.kind) {
        case "empty":
        case "control_character":
        case "invalid_url":
          return t(($) => $.errors.codes.FIELD_INVALID[problem.kind], { field })
        case "too_long":
          return t(($) => $.errors.codes.FIELD_INVALID.too_long, {
            field,
            maxChars: problem.maxChars,
          })
        case "out_of_range":
          return t(($) => $.errors.codes.FIELD_INVALID.out_of_range, {
            field,
            min: problem.min,
            max: problem.max,
          })
        default:
          return assertNever(problem)
      }
    }
    return (body: ApiErrorBody): string => {
      switch (body.code) {
        case "FIELD_INVALID":
          return fieldProblem(
            t(($) => $.errors.field[body.params.field]),
            body.params.problem,
          )
        case "NOT_FOUND":
          return t(($) => $.errors.codes.NOT_FOUND, {
            resource: t(($) => $.errors.resource[body.params.resource]),
          })
        case "REVISION_CONFLICT":
          return t(($) => $.errors.codes.REVISION_CONFLICT, {
            resource: t(($) => $.errors.resource[body.params.resource]),
          })
        case "STATE_CONFLICT":
          return t(($) => $.errors.codes.STATE_CONFLICT[body.params])
        case "ANSWER_INVALID": {
          const problem = body.params.problem
          return problem.kind === "too_long"
            ? t(($) => $.errors.codes.ANSWER_INVALID.too_long, { maxChars: problem.maxChars })
            : t(($) => $.errors.codes.ANSWER_INVALID[problem.kind])
        }
        case "CITATION_INVALID": {
          const problem = body.params.problem
          return problem.kind === "limit_exceeded"
            ? t(($) => $.errors.codes.CITATION_INVALID.limit_exceeded, { max: problem.max })
            : t(($) => $.errors.codes.CITATION_INVALID[problem.kind])
        }
        case "MODEL_NOT_READY":
          return t(($) => $.errors.codes.MODEL_NOT_READY, {
            problem: modelProblem(body.params.problem),
          })
        case "MODEL_SERVICE_FAILED":
          return t(($) => $.errors.codes.MODEL_SERVICE_FAILED, {
            failure: providerFailure(body.params.failure),
          })
        case "IDEMPOTENCY_CONFLICT":
        case "SERVICE_STOPPING":
        case "MALFORMED_REQUEST":
        case "ROUTE_NOT_FOUND":
        case "UNTRUSTED_HOST":
        case "UNTRUSTED_ORIGIN":
          return t(($) => $.errors.codes[body.code])
        case "INTERNAL":
          return `${t(($) => $.errors.codes.INTERNAL)} ${t(($) => $.errors.reference, {
            requestId: body.requestId,
          })}`
        default:
          return assertNever(body)
      }
    }
  }, [t, modelProblem, providerFailure])

  return useCallback(
    (error: unknown) => {
      if (error instanceof ApiError) return describeProblem(error.body)
      if (error instanceof NetworkError) return t(($) => $.errors.network)
      return t(($) => $.errors.unexpected)
    },
    [describeProblem, t],
  )
}
