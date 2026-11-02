/** The entity enums of spec/02 §3, shared by the table filters and the create dialog. */
import type { EntityKind } from "@/api/gen/EntityKind"
import type { EntityStatus } from "@/api/gen/EntityStatus"

export const ENTITY_KINDS: EntityKind[] = [
  "character",
  "organization",
  "location",
  "item",
  "other",
]

export const ENTITY_STATUSES: EntityStatus[] = [
  "active",
  "inactive",
  "dead",
  "destroyed",
  "missing",
  "unknown",
]
