import type { QueryClient, QueryKey } from "@tanstack/react-query"

import type { Event } from "../generated/Event"
import { queryKeys } from "../queries/keys"

/**
 * The cached reads each kind of event affects. The mapped type requires an entry for every
 * event type, so a new event cannot be forgotten.
 */
const affectedQueries: {
  [Type in Event["type"]]: (event: Extract<Event, { type: Type }>) => readonly QueryKey[]
} = {
  project_changed: () => [queryKeys.projects.all],
}

/** Refreshes the cached reads an event affects. */
export async function invalidateFor(queryClient: QueryClient, event: Event): Promise<void> {
  const affected = affectedQueries[event.type](event)
  await Promise.all(affected.map((queryKey) => queryClient.invalidateQueries({ queryKey })))
}
