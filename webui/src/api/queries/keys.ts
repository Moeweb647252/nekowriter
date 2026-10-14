import type { ProjectId } from "../generated/ProjectId"
import type { ProjectListFilter } from "../generated/ProjectListFilter"
import type { RevisionId } from "../generated/RevisionId"
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
  /** Revisions never change; no event invalidates them. */
  revision: (revisionId: RevisionId) => ["revisions", revisionId] as const,
  settings: ["settings"] as const,
  /** Everything the dashboard counts changes with workspaces, projects and model tests. */
  dashboard: {
    all: ["dashboard"] as const,
    view: (boundaries: readonly number[], project: ProjectId | null) =>
      ["dashboard", boundaries.join(","), project] as const,
  },
  health: ["health"] as const,
}
