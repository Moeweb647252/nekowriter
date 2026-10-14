import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import { useCommandIds } from "../command"
import { knowledgeApi } from "../endpoints/knowledge"
import type { CommandId } from "../generated/CommandId"
import type { EntryFields } from "../generated/EntryFields"
import type { EntryView } from "../generated/EntryView"
import type { KnowledgeEntryId } from "../generated/KnowledgeEntryId"
import type { KnowledgeSpaceId } from "../generated/KnowledgeSpaceId"
import type { SpaceView } from "../generated/SpaceView"
import { queryKeys } from "./keys"

export function useSpaces() {
  return useQuery({ queryKey: queryKeys.knowledge.spaces, queryFn: knowledgeApi.spaces })
}

export function useEntries(space: KnowledgeSpaceId | null, filter: string) {
  return useQuery({
    queryKey: queryKeys.knowledge.entries(space, filter),
    queryFn: () => knowledgeApi.entries(space, filter),
  })
}

export function useEntry(id: KnowledgeEntryId | null) {
  return useQuery({
    queryKey: queryKeys.knowledge.entry(id ?? ""),
    queryFn: () => knowledgeApi.entry(id ?? ""),
    enabled: id !== null,
  })
}

/** A knowledge command; it refreshes the knowledge reads whether it succeeds or not. */
function useKnowledgeCommand<Input, Output>(
  send: (input: Input, commandId: CommandId) => Promise<Output>,
) {
  const queryClient = useQueryClient()
  const commandIds = useCommandIds()
  return useMutation({
    mutationFn: (input: Input) => send(input, commandIds.idFor(input)),
    onSuccess: () => commandIds.succeeded(),
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.knowledge.all }),
  })
}

export function useCreateSpace() {
  return useKnowledgeCommand((name: string, commandId) =>
    knowledgeApi.createSpace({ commandId, name }),
  )
}

export function useRenameSpace() {
  return useKnowledgeCommand(({ space, name }: { space: SpaceView; name: string }, commandId) =>
    knowledgeApi.renameSpace(space.id, { commandId, expectedRevision: space.revision, name }),
  )
}

export function useDeleteSpace() {
  return useKnowledgeCommand((space: SpaceView, commandId) =>
    knowledgeApi.deleteSpace(space.id, { commandId, expectedRevision: space.revision }),
  )
}

export function useCreateEntry() {
  return useKnowledgeCommand((entry: EntryFields, commandId) =>
    knowledgeApi.createEntry({ commandId, ...entry }),
  )
}

export function useUpdateEntry() {
  return useKnowledgeCommand(
    ({ entry, fields }: { entry: EntryView; fields: EntryFields }, commandId) =>
      knowledgeApi.updateEntry(entry.id, {
        commandId,
        expectedRevision: entry.revision,
        ...fields,
      }),
  )
}

export function useDeleteEntry() {
  return useKnowledgeCommand((entry: EntryView, commandId) =>
    knowledgeApi.deleteEntry(entry.id, { commandId, expectedRevision: entry.revision }),
  )
}
