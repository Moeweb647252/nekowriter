/** Narrowing helpers for API errors (no `any`, spec/11 conventions). */
import { ApiError } from "@/api/client"

/** True when a thrown value is an `ApiError` from the fetch wrapper. */
export function isApiError(error: unknown): error is ApiError {
  return error instanceof ApiError
}

/** The `details` payload of an error, when present. */
export function errorDetails(error: unknown): Record<string, unknown> | null {
  return isApiError(error) ? error.details : null
}

/** A numeric field of an error's `details`. */
export function errorDetailNumber(error: unknown, key: string): number | null {
  const details = errorDetails(error)
  const value = details?.[key]
  return typeof value === "number" ? value : null
}
