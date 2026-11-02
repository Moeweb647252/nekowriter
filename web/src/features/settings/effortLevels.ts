/** Thinking-effort rules shared by the Models dialog and the stored-mismatch UI (D-162).
 *
 * The server owns validation (`EffortRule` in `crates/llm`); these helpers mirror the two rules
 * the dialog must enforce before a save:
 *  - the level token pattern (`^[A-Za-z0-9_-]{1,32}$`);
 *  - which levels a protocol can send: Anthropic has the closed canonical set, OpenAI-compatible
 *    providers accept any token matching the pattern, and the Demo provider sends no request at
 *    all.
 *
 * The suggestions are the preset defaults from `GET /provider-presets` (spec/04 §8); free text is
 * still allowed on OpenAI because gateways accept different names.
 */
import type { Protocol } from "@/api/gen/Protocol"

/** At most this many levels fit in one profile (spec/04 §5.1). */
export const MAX_EFFORT_LEVELS = 8

/** The server's level token pattern, mirrored for the dialog's local checks. */
export const EFFORT_LEVEL_PATTERN = /^[A-Za-z0-9_-]{1,32}$/

/** Anthropic's closed, canonical effort set. */
export const ANTHROPIC_EFFORT_LEVELS = ["low", "medium", "high", "xhigh", "max"] as const

/** The levels the dialog offers as suggestions, by protocol. */
export const EFFORT_SUGGESTIONS: Record<Protocol, readonly string[]> = {
  anthropic: ANTHROPIC_EFFORT_LEVELS,
  openai: ["low", "medium", "high"],
  mock: [],
}

export type EffortAddError = "invalid" | "duplicate" | "limit"

export type EffortAddResult = {
  levels: string[]
  error: EffortAddError | null
}

/** Applies the server's add rules to one raw combobox value. */
export function addEffortLevel(
  levels: readonly string[],
  raw: string,
): EffortAddResult {
  const value = raw.trim()
  if (value === "") {
    return { levels: [...levels], error: null }
  }
  if (!EFFORT_LEVEL_PATTERN.test(value)) {
    return { levels: [...levels], error: "invalid" }
  }
  if (levels.includes(value)) {
    return { levels: [...levels], error: "duplicate" }
  }
  if (levels.length >= MAX_EFFORT_LEVELS) {
    return { levels: [...levels], error: "limit" }
  }
  return { levels: [...levels, value], error: null }
}

/** Moves one level, for the dnd-kit drop handler. */
export function reorderEffortLevels(
  levels: readonly string[],
  oldIndex: number,
  newIndex: number,
): string[] {
  if (
    oldIndex < 0 ||
    newIndex < 0 ||
    oldIndex >= levels.length ||
    newIndex >= levels.length ||
    oldIndex === newIndex
  ) {
    return [...levels]
  }
  const next = [...levels]
  const [moved] = next.splice(oldIndex, 1)
  next.splice(newIndex, 0, moved)
  return next
}

/** Whether one level is sendable by `protocol`. */
export function isSendableEffortLevel(
  protocol: Protocol | null,
  level: string,
): boolean {
  if (protocol === "openai") {
    return EFFORT_LEVEL_PATTERN.test(level)
  }
  if (protocol === "anthropic") {
    return (ANTHROPIC_EFFORT_LEVELS as readonly string[]).includes(level)
  }
  return false
}

/** The levels of `levels` that `protocol` cannot send, in their stored order. */
export function unsendableEffortLevels(
  levels: readonly string[],
  protocol: Protocol | null,
): string[] {
  if (protocol === null) {
    return []
  }
  return levels.filter((level) => !isSendableEffortLevel(protocol, level))
}

/** The suggestions for one protocol, minus the levels already present. */
export function effortSuggestionsFor(
  protocol: Protocol | null,
  levels: readonly string[],
): string[] {
  if (protocol === null) {
    return []
  }
  return EFFORT_SUGGESTIONS[protocol].filter((level) => !levels.includes(level))
}
