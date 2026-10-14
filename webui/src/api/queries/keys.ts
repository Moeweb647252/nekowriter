import type { KnowledgeEntryId } from "../generated/KnowledgeEntryId"
import type { KnowledgeSpaceId } from "../generated/KnowledgeSpaceId"
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
  embeddingSettings: ["settings", "embedding"] as const,
  templates: ["templates"] as const,
  knowledge: {
    all: ["knowledge"] as const,
    spaces: ["knowledge", "spaces"] as const,
    entries: (space: KnowledgeSpaceId | null, filter: string) =>
      ["knowledge", "entries", space, filter] as const,
    entry: (id: KnowledgeEntryId) => ["knowledge", "entry", id] as const,
  },
  /** Prompts change with the settings (system overrides) and projects (theirs). */
  prompts: {
    all: ["prompts"] as const,
    scope: (project: ProjectId | null) => ["prompts", project] as const,
  },
  /** Everything the dashboard counts changes with workspaces, projects and model tests. */
  dashboard: {
    all: ["dashboard"] as const,
    view: (boundaries: readonly number[], project: ProjectId | null) =>
      ["dashboard", boundaries.join(","), project] as const,
  },
  health: ["health"] as const,
}
