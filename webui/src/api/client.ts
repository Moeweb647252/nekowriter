/**
 * The HTTP client for the NekoWriter API. Requests and responses use the types generated
 * from the Rust definitions in `./generated`.
 */
import type { ApiErrorBody } from "./generated/ApiErrorBody"

/** The API answered with an error body. */
export class ApiError extends Error {
  readonly status: number
  readonly body: ApiErrorBody

  constructor(status: number, body: ApiErrorBody) {
    super(`${body.code} (request ${body.requestId})`)
    this.name = "ApiError"
    this.status = status
    this.body = body
  }
}

/** The service could not be reached, or something other than the API answered. */
export class NetworkError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options)
    this.name = "NetworkError"
  }
}

type QueryParams = Record<string, string | number | undefined>

export function getJson<T>(path: string, query?: QueryParams): Promise<T> {
  return send<T>("GET", withQuery(path, query))
}

export function postJson<T>(path: string, body: unknown): Promise<T> {
  return send<T>("POST", path, body)
}

function withQuery(path: string, query: QueryParams | undefined): string {
  const params = new URLSearchParams()
  for (const [key, value] of Object.entries(query ?? {})) {
    if (value !== undefined) params.set(key, String(value))
  }
  const search = params.toString()
  return search ? `${path}?${search}` : path
}

async function send<T>(method: string, path: string, body?: unknown): Promise<T> {
  let response: Response
  try {
    response = await fetch(new URL(path, window.location.href), {
      method,
      ...(body === undefined
        ? {}
        : { headers: { "content-type": "application/json" }, body: JSON.stringify(body) }),
    })
  } catch (cause) {
    throw new NetworkError(`${method} ${path} failed`, { cause })
  }
  if (!response.headers.get("content-type")?.includes("application/json")) {
    throw new NetworkError(`${method} ${path} answered ${response.status} without JSON`)
  }
  const payload: unknown = await response.json()
  if (!response.ok) throw new ApiError(response.status, payload as ApiErrorBody)
  return payload as T
}
