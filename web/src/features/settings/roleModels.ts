/**
 * Shared helpers for the role → model pickers (PROJECT §8.12, spec/03 §4 step 1).
 *
 * Two screens assign a model profile to an agent role: the global Agent roles page, which writes
 * `app_settings["roles.defaults"]`, and Book settings, which overrides one or more roles for a
 * single book (`books.model_overrides`, resolved before the global default). Both need the same
 * five roles in the same order, the same role labels, and the same provider-qualified model name
 * ("Provider · Model"), including the "missing" case for a profile that was deleted after it was
 * assigned. Keeping the formatting here means the two pages cannot drift apart.
 */
import type { TFunction } from "i18next"

import type { AgentRole } from "@/api/gen/AgentRole"
import type { ModelProfileDto } from "@/api/gen/ModelProfileDto"
import type { ProviderDto } from "@/api/gen/ProviderDto"
import type { RoleModels } from "@/api/gen/RoleModels"

/** The five roles in the order the settings pages list them (spec/02 §3). */
export const AGENT_ROLES: readonly AgentRole[] = [
  "conception",
  "researcher",
  "writer",
  "reviewer",
  "archivist",
]

/** The role → i18n key of its label (spec/11 §6). Both settings screens reuse these keys. */
const ROLE_LABEL_KEYS: Record<AgentRole, string> = {
  conception: "settings:role.conception",
  researcher: "settings:role.researcher",
  writer: "settings:role.writer",
  reviewer: "settings:role.reviewer",
  archivist: "settings:role.archivist",
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
    writer: models?.writer ?? null,
    reviewer: models?.reviewer ?? null,
    archivist: models?.archivist ?? null,
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
