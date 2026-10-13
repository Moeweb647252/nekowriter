import { ApiError } from "./client"
import type { Field } from "./generated/Field"

/**
 * Splits a command's error between form fields: a validation error that names `field`
 * belongs under that field; every other error belongs to the form as a whole.
 */
export function errorForField(error: unknown, field: Field): unknown {
  return isFieldError(error, field) ? error : undefined
}

export function errorForForm(error: unknown, fields: readonly Field[]): unknown {
  if (error == null) return undefined
  return fields.some((field) => isFieldError(error, field)) ? undefined : error
}

function isFieldError(error: unknown, field: Field): boolean {
  return (
    error instanceof ApiError &&
    error.body.code === "FIELD_INVALID" &&
    error.body.params.field === field
  )
}
