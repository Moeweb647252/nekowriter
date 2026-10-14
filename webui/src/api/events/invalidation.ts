import type { QueryClient, QueryKey } from "@tanstack/react-query"

import type { Event } from "../generated/Event"
import { queryKeys } from "../queries/keys"
import { assertNever } from "@/lib/assert"

/**
 * The cached reads an event affects. The switch covers every event type, so a new event
 * cannot be forgotten.
 */
function affectedQueries(event: Event): readonly QueryKey[] {
  switch (event.type) {
    // Workspaces show their project and its versions.
    case "project_changed":
      return [
        queryKeys.projects.all,
        queryKeys.workspaces.all,
        queryKeys.dashboard.all,
        queryKeys.prompts.all,
      ]
    // Project lists show the selected version's stage.
    case "version_changed":
      return [
        queryKeys.workspaces.version(event.versionId),
        queryKeys.projects.all,
        queryKeys.dashboard.all,
      ]
    case "settings_changed":
      return [queryKeys.settings, queryKeys.prompts.all]
    case "templates_changed":
      return [queryKeys.templates]
    case "knowledge_changed":
      return [queryKeys.knowledge.all]
    default:
      return assertNever(event)
  }
}

/** Refreshes the cached reads an event affects. */
export async function invalidateFor(queryClient: QueryClient, event: Event): Promise<void> {
  await Promise.all(
    affectedQueries(event).map((queryKey) => queryClient.invalidateQueries({ queryKey })),
  )
}
