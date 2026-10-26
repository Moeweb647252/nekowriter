/** Protocol rules for the model-option form (spec/04 §5, D-132).
 *
 * The rule "which option can this protocol send" lives in `crates/llm` and is served by
 * `GET /api/provider-presets/options`, so this module never guesses: it only intersects the
 * profile's options with the served table. The bug this replaces: the form offered `effort`
 * (Anthropic) while the provider spoke the OpenAI protocol, the OpenAI request carries only
 * `reasoning_effort`, and the stored value silently never reached the model.
 */
import type { ModelOptionsDto } from "@/api/gen/ModelOptionsDto"
import type { ModelOptionSpecDto } from "@/api/gen/ModelOptionSpecDto"
import type { Protocol } from "@/api/gen/Protocol"
/** The part of a provider row the option form needs: its protocol decides what it can send. */
export type ProtocolCarrier = { id: number; protocol: Protocol }

/** The `Select` value that stands for "leave this option unset" (D-133).
 *
 * Radix's `SelectValue` renders its **placeholder** branch for an empty-string or `undefined`
 * value (`shouldShowPlaceholder`), and the models form passes no `placeholder`, so an
 * empty-string item rendered nothing at all: the Add-model dialog showed blank selects while the
 * option table knew every default. The unset state is a real, selectable, persistent choice, not a
 * hint, so it needs a value of its own — {@link fromOptionSelection} maps it back to `undefined`.
 *
 * The sentinel must stay outside every served `values` list; a test asserts that, so a future
 * option value cannot collide with it.
 */
export const DEFAULT_OPTION_VALUE = "__default__"

/** The `Select` value of an option: the value itself, or {@link DEFAULT_OPTION_VALUE} when unset.
 *
 * Never returns `""`, which Radix treats as "no value" and renders as an empty trigger. A stored
 * empty string is treated as unset too: it is not one of the option's published values, and
 * displaying it would recreate exactly the blank control this mapping exists to prevent.
 */
export function toOptionSelection(value: string | null | undefined): string {
  return value === undefined || value === null || value === ""
    ? DEFAULT_OPTION_VALUE
    : value
}

/** The option value a `Select` value stands for: the sentinel (or an empty string) means "unset",
 * i.e. `undefined`, which the dialog writes by deleting the key. */
export function fromOptionSelection(selection: string): string | undefined {
  return selection === DEFAULT_OPTION_VALUE || selection === "" ? undefined : selection
}

/** The option keys this UI edits, in the order the server publishes them. */
export const OPTION_KEYS = [
  "reasoning_replay",
  "cache_ttl",
  "reasoning_effort",
  "max_tokens_field",
  "send_parallel_tool_calls",
  "stream_usage",
  "temperature",
  "extra_body",
  "effort",
  "thinking_display",
  "refusal_fallback",
  "eager_input_streaming",
] as const satisfies readonly (keyof ModelOptionsDto)[]

export type OptionKey = (typeof OPTION_KEYS)[number]

/** The protocol of a provider row, or `null` when the provider is not loaded (yet). */
export function providerProtocol(
  providers: readonly ProtocolCarrier[],
  providerId: string | number,
): Protocol | null {
  const id = typeof providerId === "number" ? providerId : Number(providerId)
  if (!Number.isFinite(id)) {
    return null
  }
  return providers.find((provider) => provider.id === id)?.protocol ?? null
}

/** The options a protocol can send, in `ModelOptions` order. */
export function optionsForProtocol(
  specs: readonly ModelOptionSpecDto[],
  protocol: Protocol | null,
): ModelOptionSpecDto[] {
  // Without a protocol (no provider selected) the form shows nothing: offering an option that
  // might not apply is what produced the bug report in the first place.
  if (protocol === null) {
    return []
  }
  return specs.filter((spec) => spec.protocols.includes(protocol))
}

/** The keys of `options` that `protocol` cannot send, in table order.
 *
 * The server reports exactly this set in `ModelProfileDto.ignored_options`, and the two must agree:
 * a value equal to the option's `default_value` is not "set", because `ModelOptions` initializes
 * every field and the stored blob therefore carries defaults too. Reporting those would flag
 * profiles that are fine.
 */
export function unreachableOptionKeys(
  options: ModelOptionsDto,
  specs: readonly ModelOptionSpecDto[],
  protocol: Protocol | null,
): OptionKey[] {
  if (protocol === null) {
    return []
  }
  const reachable = new Set(optionsForProtocol(specs, protocol).map((spec) => spec.key))
  const defaults = new Map(specs.map((spec) => [spec.key, spec.default_value]))
  return OPTION_KEYS.filter((key) => {
    const value = options[key]
    if (!isSet(value)) {
      return false
    }
    return !reachable.has(key) && !isDefault(value, defaults.get(key))
  }) as OptionKey[]
}

/** Whether a value is the option's effective default, which the server does not treat as set. */
function isDefault(value: unknown, defaultValue: string | null | undefined): boolean {
  return defaultValue !== null && defaultValue !== undefined && String(value) === defaultValue
}

/** Removes the options a protocol cannot send, returning what was removed.
 *
 * The settings form calls this when the author switches a profile to another provider: the
 * stranded fields are removed from the *form* (never silently on the server) and the removal is
 * shown, so a protocol change cannot recreate the original silent no-op.
 */
export function clearUnreachableOptions(
  options: ModelOptionsDto,
  specs: readonly ModelOptionSpecDto[],
  protocol: Protocol | null,
): { options: ModelOptionsDto; removed: OptionKey[] } {
  const removed = unreachableOptionKeys(options, specs, protocol)
  if (removed.length === 0) {
    return { options, removed }
  }
  const next: ModelOptionsDto = { ...options }
  for (const key of removed) {
    delete next[key]
  }
  return { options: next, removed }
}

/** Removes exactly `keys` from an options blob (the stored-mismatch cleanup). */
export function removeOptionKeys(
  options: ModelOptionsDto,
  keys: readonly string[],
): ModelOptionsDto {
  const next: ModelOptionsDto = { ...options }
  for (const key of keys) {
    if ((OPTION_KEYS as readonly string[]).includes(key)) {
      delete next[key as OptionKey]
    }
  }
  return next
}

/** Whether an option carries a value. Every field is optional, and an explicit `null` is "unset"
 * too — the DTO round-trips `null` for an omitted field. */
function isSet(value: unknown): boolean {
  if (value === undefined || value === null) {
    return false
  }
  // An empty object sets nothing: `extra_body: {}` sends no field.
  return !(typeof value === "object" && !Array.isArray(value) && Object.keys(value).length === 0)
}

/** The option label key in the `settings` namespace. */
export function optionLabelKey(key: string): string {
  return `settings:option.${key}`
}

/** The short explanation key of an option in the `settings` namespace. */
export function optionHintKey(key: string): string {
  return `settings:option.${key}_hint`
}

/** The label key of a closed enum value, with the raw value as the fallback for values the
 * locales do not name. */
export function optionValueKey(key: string, value: string): string {
  return `settings:value.${key}.${value}`
}
