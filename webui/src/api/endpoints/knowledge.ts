import { getJson, postJson } from "../client"
import type { CreateEntryRequest } from "../generated/CreateEntryRequest"
import type { CreateSpaceRequest } from "../generated/CreateSpaceRequest"
import type { EntrySummary } from "../generated/EntrySummary"
import type { EntryView } from "../generated/EntryView"
import type { KnowledgeEntryId } from "../generated/KnowledgeEntryId"
import type { KnowledgeSpaceId } from "../generated/KnowledgeSpaceId"
import type { RenameSpaceRequest } from "../generated/RenameSpaceRequest"
import type { RevisionRequest } from "../generated/RevisionRequest"
import type { SpaceView } from "../generated/SpaceView"
import type { UpdateEntryRequest } from "../generated/UpdateEntryRequest"

export const knowledgeApi = {
  spaces: () => getJson<SpaceView[]>("/api/knowledge/spaces"),
  createSpace: (request: CreateSpaceRequest) =>
    postJson<SpaceView>("/api/knowledge/spaces", request),
  renameSpace: (id: KnowledgeSpaceId, request: RenameSpaceRequest) =>
    postJson<SpaceView>(`/api/knowledge/spaces/${id}/rename`, request),
  deleteSpace: (id: KnowledgeSpaceId, request: RevisionRequest) =>
    postJson<SpaceView>(`/api/knowledge/spaces/${id}/delete`, request),
  entries: (space: KnowledgeSpaceId | null, filter: string) =>
    getJson<EntrySummary[]>("/api/knowledge/entries", {
      space: space ?? undefined,
      filter: filter === "" ? undefined : filter,
    }),
  entry: (id: KnowledgeEntryId) => getJson<EntryView>(`/api/knowledge/entries/${id}`),
  createEntry: (request: CreateEntryRequest) =>
    postJson<EntryView>("/api/knowledge/entries", request),
  updateEntry: (id: KnowledgeEntryId, request: UpdateEntryRequest) =>
    postJson<EntryView>(`/api/knowledge/entries/${id}/update`, request),
  deleteEntry: (id: KnowledgeEntryId, request: RevisionRequest) =>
    postJson<EntryView>(`/api/knowledge/entries/${id}/delete`, request),
}
