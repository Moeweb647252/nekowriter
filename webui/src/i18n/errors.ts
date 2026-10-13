import { useCallback } from "react"
import { useTranslation } from "react-i18next"

import { ApiError, NetworkError } from "@/api/client"
import type { ApiErrorBody } from "@/api/generated/ApiErrorBody"
import type { ApiProblem } from "@/api/generated/ApiProblem"
import type { Field } from "@/api/generated/Field"
import type { FieldProblem } from "@/api/generated/FieldProblem"
import type { Resource } from "@/api/generated/Resource"
import type { StateConflict } from "@/api/generated/StateConflict"
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
  codes: {
    [Code in ApiProblem["code"]]: Code extends "FIELD_INVALID"
      ? Record<FieldProblem["kind"], string>
      : Code extends "STATE_CONFLICT"
        ? Record<StateConflict, string>
        : string
  }
}

/** Returns a function that describes any thrown error in the interface language. */
export function useErrorText(): (error: unknown) => string {
  const { t } = useTranslation()

  const describeProblem = useCallback(
    (body: ApiErrorBody): string => {
      switch (body.code) {
        case "FIELD_INVALID": {
          const field = t(($) => $.errors.field[body.params.field])
          const problem = body.params.problem
          switch (problem.kind) {
            case "empty":
              return t(($) => $.errors.codes.FIELD_INVALID.empty, { field })
            case "too_long":
              return t(($) => $.errors.codes.FIELD_INVALID.too_long, {
                field,
                maxChars: problem.maxChars,
              })
            case "control_character":
              return t(($) => $.errors.codes.FIELD_INVALID.control_character, { field })
            default:
              return assertNever(problem)
          }
        }
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
        case "IDEMPOTENCY_CONFLICT":
          return t(($) => $.errors.codes.IDEMPOTENCY_CONFLICT)
        case "MALFORMED_REQUEST":
          return t(($) => $.errors.codes.MALFORMED_REQUEST)
        case "ROUTE_NOT_FOUND":
          return t(($) => $.errors.codes.ROUTE_NOT_FOUND)
        case "UNTRUSTED_HOST":
          return t(($) => $.errors.codes.UNTRUSTED_HOST)
        case "UNTRUSTED_ORIGIN":
          return t(($) => $.errors.codes.UNTRUSTED_ORIGIN)
        case "INTERNAL":
          return `${t(($) => $.errors.codes.INTERNAL)} ${t(($) => $.errors.reference, {
            requestId: body.requestId,
          })}`
        default:
          return assertNever(body)
      }
    },
    [t],
  )

  return useCallback(
    (error: unknown) => {
      if (error instanceof ApiError) return describeProblem(error.body)
      if (error instanceof NetworkError) return t(($) => $.errors.network)
      return t(($) => $.errors.unexpected)
    },
    [describeProblem, t],
  )
}
