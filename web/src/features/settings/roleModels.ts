/**
 * Shared helpers for the role → model pickers (PROJECT §8.12, spec/03 §4 step 1).
 *
 * Two screens assign a model profile to an agent role: the global Agent roles page, which writes
 * `app_settings["roles.defaults"]`, and Book settings, which overrides one or more roles for a
 * single book (`books.model_overrides`, resolved before the global default). Both need the same
 * six roles in the same order, the same role labels, and the same provider-qualified model name
 * ("Provider · Model"), including the "missing" case for a profile that was deleted after it was
 * assigned. Keeping the formatting here means the two pages cannot drift apart.
 */
import type { TFunction } from "i18next"

import type { AgentRole } from "@/api/gen/AgentRole"
import type { ModelProfileDto } from "@/api/gen/ModelProfileDto"
import type { ProviderDto } from "@/api/gen/ProviderDto"
import type { RoleEfforts } from "@/api/gen/RoleEfforts"
import type { RoleModels } from "@/api/gen/RoleModels"

import { isSendableEffortLevel } from "@/features/settings/effortLevels"

/** The seven roles in the order the settings pages list them (spec/02 §3, D-146, D-169). */
export const AGENT_ROLES: readonly AgentRole[] = [
  "conception",
  "researcher",
  "planner",
  "writer",
  "reviewer",
  "archivist",
  "assistant",
]

/** The role → i18n key of its label (spec/11 §6). Both settings screens reuse these keys. */
const ROLE_LABEL_KEYS: Record<AgentRole, string> = {
  conception: "settings:role.conception",
  researcher: "settings:role.researcher",
  planner: "settings:role.planner",
  writer: "settings:role.writer",
  reviewer: "settings:role.reviewer",
  archivist: "settings:role.archivist",
  assistant: "settings:role.assistant",
}

/** The localized label key of one agent role. */
export function roleLabelKey(role: AgentRole): string {
  return ROLE_LABEL_KEYS[role]
}

/**
 * One explicit entry per role, with `null` meaning "use the global default". The API type keeps
 * the fields optional (`RoleModels`), so normalizing here lets a page index it by role without a
 * `?? null` at every use.
 */
export type RoleSelection = Record<AgentRole, number | null>

/** Normalizes the API's optional `RoleModels` object into one entry per role. */
export function roleSelection(models: RoleModels | null | undefined): RoleSelection {
  return {
    conception: models?.conception ?? null,
    researcher: models?.researcher ?? null,
    planner: models?.planner ?? null,
    writer: models?.writer ?? null,
    reviewer: models?.reviewer ?? null,
    archivist: models?.archivist ?? null,
    assistant: models?.assistant ?? null,
  }
}

/**
 * The label of a model profile: "Provider · Model" (a role means nothing without the provider it
 * runs on), the bare display name when the provider row is gone, or the localized "Missing model"
 * when the profile itself no longer exists — an assigned-then-deleted profile must never render as
 * an empty option.
 */
export function formatModelLabel(
  t: TFunction,
  models: readonly ModelProfileDto[],
  providers: readonly ProviderDto[],
  modelId: number,
): string {
  const model = models.find((candidate) => candidate.id === modelId)
  if (model === undefined) {
    return t("settings:model_missing")
  }
  const provider = providers.find((candidate) => candidate.id === model.provider_id)
  // The provider name and the display name are author data, not translated fragments; the
  // separator is punctuation. Only the missing case has localized text.
  return provider === undefined
    ? model.display_name
    : `${provider.name} · ${model.display_name}`
}

/** The inputs of the pure role-effort resolver (mirrors spec/03 §4 step 1 and plan §4.4). */
export type RoleEffortInputs = {
  role: AgentRole
  /** The book's `model_overrides` (null on the global Settings › Agent roles page). */
  book: RoleModels | null
  global: RoleModels
  models: readonly ModelProfileDto[]
  providers: readonly ProviderDto[]
}

/** What one role row needs to label its effort select and describe a stale choice. */
export type RoleEffortResolution = {
  /** The model the role resolves to, or null when no model can be resolved. */
  model: ModelProfileDto | null
  /** The model's accepted levels, in display order (empty when there is no model). */
  levels: string[]
  /** The model's default level, or null when the provider decides. */
  defaultEffort: string | null
  /** The role's own stored choice at this scope (book choice on Book settings, global otherwise). */
  storedEffort: string | null
  /** What session creation would actually use. */
  effectiveEffort: string | null
  /** True only when the planner has no own model and follows the writer. */
  modelFromWriter: boolean
  /** True when `storedEffort` is not one of the resolved model's levels, or is not sendable
   * by the model's protocol. */
  stale: boolean
}

/**
 * The pure resolver behind both role pages (spec/03 §4 step 1, plan §4.4).
 *
 * Model: `book.models[role]` → `global.models[role]`; the planner additionally follows
 * `book.models.writer` → `global.models.writer`, signalled by `modelFromWriter`.
 *
 * Effort candidates: `book.efforts[role]` → `global.efforts[role]`, then the writer's book/global
 * entries only when the planner follows the writer's model. The first candidate that is one of the
 * resolved model's levels and sendable by its protocol wins; otherwise the model default applies
 * when sendable; otherwise there is no effective effort. A stored choice outside the model's levels
 * is marked stale.
 */
export function resolveRoleEffort({
  role,
  book,
  global,
  models,
  providers,
}: RoleEffortInputs): RoleEffortResolution {
  const bookModels = book
  const ownModel = bookModels?.[role] ?? global[role] ?? null
  let modelId = ownModel
  let modelFromWriter = false
  if (modelId === null && role === "planner") {
    const writerModel = bookModels?.writer ?? global.writer ?? null
    if (writerModel !== null) {
      modelId = writerModel
      modelFromWriter = true
    }
  }
  const model = modelId === null ? null : (models.find((candidate) => candidate.id === modelId) ?? null)
  const levels = model?.effort_levels ?? []
  const defaultEffort = model?.default_effort ?? null
  const protocol =
    model === null
      ? null
      : (providers.find((provider) => provider.id === model.provider_id)?.protocol ?? null)
  const sendable = (level: string) =>
    levels.includes(level) && isSendableEffortLevel(protocol, level)

  const chosen = (efforts: RoleEfforts | null | undefined, candidateRole: AgentRole) =>
    efforts?.[candidateRole] ?? null
  const candidates = [
    chosen(book?.efforts, role),
    chosen(global.efforts, role),
    ...(modelFromWriter
      ? [chosen(book?.efforts, "writer"), chosen(global.efforts, "writer")]
      : []),
  ].filter((candidate): candidate is string => candidate !== null)

  let effectiveEffort: string | null = null
  for (const candidate of candidates) {
    if (sendable(candidate)) {
      effectiveEffort = candidate
      break
    }
  }
  if (effectiveEffort === null && defaultEffort !== null && sendable(defaultEffort)) {
    effectiveEffort = defaultEffort
  }

  const storedEffort = chosen(book?.efforts ?? global.efforts, role)
  const stale = storedEffort !== null && !sendable(storedEffort)
  return {
    model,
    levels,
    defaultEffort,
    storedEffort,
    effectiveEffort,
    modelFromWriter,
    stale,
  }
}
