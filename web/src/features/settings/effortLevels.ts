/** Thinking-effort rules shared by the Models dialog and the stored-mismatch UI (D-162).
 *
 * The server owns validation (`EffortRule` in `crates/llm`); these helpers mirror the two rules
 * the dialog must enforce before a save:
 *  - the level token pattern (`^[A-Za-z0-9_-]{1,32}$`);
 *  - which levels a protocol can send: Anthropic has the closed canonical set, OpenAI-compatible
 *    providers accept any token matching the pattern, and the Demo provider sends no request at
 *    all.
 *
 * Anthropic uses its canonical set as checkboxes; OpenAI-compatible providers accept free text
 * because gateways accept different names.
 */
import type { Protocol } from "@/api/gen/Protocol"

/** At most this many levels fit in one profile (spec/04 §5.1). */
export const MAX_EFFORT_LEVELS = 8

/** The server's level token pattern, mirrored for the dialog's local checks. */
export const EFFORT_LEVEL_PATTERN = /^[A-Za-z0-9_-]{1,32}$/

/** Anthropic's closed, canonical effort set. */
export const ANTHROPIC_EFFORT_LEVELS = ["low", "medium", "high", "xhigh", "max"] as const

export type EffortAddError = "invalid" | "duplicate" | "limit"

export type EffortAddResult = {
  levels: string[]
  error: EffortAddError | null
}

/** Applies the server's add rules to one raw chip-input value. */
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
