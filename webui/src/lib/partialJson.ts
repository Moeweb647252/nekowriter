import { Allow, parse } from "partial-json"

/** A JSON object read from text that may still be incomplete. */
export type PartialObject = Record<string, unknown>

/**
 * Reads the object streamed so far. Anything that does not parse yet, or is not an
 * object, reads as empty until more of it arrives.
 */
export function readPartialObject(json: string): PartialObject {
  try {
    const value: unknown = parse(json, Allow.ALL)
    return typeof value === "object" && value !== null && !Array.isArray(value)
      ? (value as PartialObject)
      : {}
  } catch {
    return {}
  }
}

export function textOf(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined
}

export function objectsOf(value: unknown): PartialObject[] {
  return Array.isArray(value)
    ? value.filter(
        (item): item is PartialObject =>
          typeof item === "object" && item !== null && !Array.isArray(item),
      )
    : []
}
