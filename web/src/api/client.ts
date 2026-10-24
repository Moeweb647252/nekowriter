/** The fetch wrapper (spec/11 §4.1). */

export type ApiErrorDetails = Record<string, unknown> | null

export class ApiError extends Error {
  readonly status: number
  readonly code: string
  readonly details: ApiErrorDetails

  constructor(status: number, code: string, message: string, details: ApiErrorDetails = null) {
    super(message)
    this.name = "ApiError"
    this.status = status
    this.code = code
    this.details = details
  }
}

type Method = "GET" | "POST" | "PATCH" | "PUT" | "DELETE"

const BASE = "/api"

/** Performs one API request and parses the JSON response. */
export async function request<T>(
  method: Method,
  path: string,
  body?: unknown,
): Promise<T> {
  let response: Response
  try {
    response = await fetch(`${BASE}${path}`, {
      method,
      headers: body === undefined ? undefined : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
  } catch (cause) {
    throw new ApiError(0, "network", cause instanceof Error ? cause.message : "network error")
  }

  if (response.status === 204) {
    return undefined as T
  }

  const text = await response.text()
  const parsed: unknown = text.length > 0 ? safeJson(text) : null

  if (!response.ok) {
    const error = extractError(parsed)
    throw new ApiError(
      response.status,
      error?.code ?? "internal",
      error?.message ?? `request failed with status ${response.status}`,
      error?.details ?? null,
    )
  }

  return parsed as T
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text) as unknown
  } catch {
    return null
  }
}

function extractError(
  parsed: unknown,
): { code?: string; message?: string; details?: ApiErrorDetails } | null {
  if (typeof parsed !== "object" || parsed === null) {
    return null
  }
  const error = (parsed as { error?: unknown }).error
  if (typeof error !== "object" || error === null) {
    return null
  }
  const record = error as Record<string, unknown>
  return {
    code: typeof record.code === "string" ? record.code : undefined,
    message: typeof record.message === "string" ? record.message : undefined,
    details:
      typeof record.details === "object" && record.details !== null
        ? (record.details as Record<string, unknown>)
        : null,
  }
}

export const api = {
  get: <T>(path: string) => request<T>("GET", path),
  post: <T>(path: string, body?: unknown) => request<T>("POST", path, body),
  patch: <T>(path: string, body?: unknown) => request<T>("PATCH", path, body),
  put: <T>(path: string, body?: unknown) => request<T>("PUT", path, body),
  delete: <T>(path: string, body?: unknown) => request<T>("DELETE", path, body),
}
