import type { ProjectListFilter } from "../generated/ProjectListFilter"
import type { VersionId } from "../generated/VersionId"

/**
 * Query keys. Every cached server read is keyed here, so the event stream can invalidate
 * exactly what an event affects (see `../events/invalidation.ts`).
 */
export const queryKeys = {
  projects: {
    all: ["projects"] as const,
    list: (filter: ProjectListFilter) => ["projects", "list", filter] as const,
  },
  workspaces: {
    all: ["workspaces"] as const,
    version: (versionId: VersionId) => ["workspaces", versionId] as const,
  },
  settings: ["settings"] as const,
  health: ["health"] as const,
}
