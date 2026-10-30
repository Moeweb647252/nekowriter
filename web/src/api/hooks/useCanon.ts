/** Canon queries and mutations (spec/11 §4.1, spec/10 §2.4). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/canon"
import type { ChangeQueryParams } from "@/api/endpoints/canon"
import { keys } from "@/api/keys"
import type { EntityInput } from "@/api/gen/EntityInput"
import type { EntityPatch } from "@/api/gen/EntityPatch"
import type { ForeshadowingInput } from "@/api/gen/ForeshadowingInput"
import type { ForeshadowingPatch } from "@/api/gen/ForeshadowingPatch"
import type { MemoryNoteInput } from "@/api/gen/MemoryNoteInput"
import type { MemoryNotePatch } from "@/api/gen/MemoryNotePatch"
import type { RelationInput } from "@/api/gen/RelationInput"
import type { RelationPatch } from "@/api/gen/RelationPatch"
import type { TimelineEventInput } from "@/api/gen/TimelineEventInput"
import type { TimelineEventPatch } from "@/api/gen/TimelineEventPatch"

/** Everything a canon write can invalidate (spec/10 §3 `canon.changed`). */
function useCanonInvalidator(bookId: number) {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: keys.canon.all })
    void queryClient.invalidateQueries({ queryKey: keys.changes.all })
    void queryClient.invalidateQueries({
      queryKey: keys.books.overview(bookId),
    })
  }
}

// ───────────────────────────── entities ─────────────────────────────

export function useEntities(
  bookId: number,
  params: { kind?: string; status?: string; q?: string; trackId?: number } = {}
) {
  const filters = [params.kind, params.status, params.q, params.trackId].join(
    "|"
  )
  return useQuery({
    queryKey: keys.canon.entities(bookId, filters),
    queryFn: () => endpoints.listEntities(bookId, params),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

export function useEntity(entityId: number | undefined) {
  return useQuery({
    queryKey: keys.canon.entity(entityId ?? 0),
    queryFn: () => endpoints.getEntity(entityId ?? 0),
    enabled:
      entityId !== undefined && Number.isFinite(entityId) && entityId > 0,
  })
}

export function useCreateEntity(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: (input: EntityInput) => endpoints.createEntity(bookId, input),
    onSuccess: invalidate,
  })
}

export function usePatchEntity(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: ({ id, patch }: { id: number; patch: EntityPatch }) =>
      endpoints.patchEntity(id, patch),
    onSuccess: invalidate,
  })
}

export function useDeleteEntity(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: (id: number) => endpoints.deleteEntity(id),
    onSuccess: invalidate,
  })
}

export function useMergeEntity(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: ({ id, fromId }: { id: number; fromId: number }) =>
      endpoints.mergeEntity(id, { from_id: fromId }),
    onSuccess: invalidate,
  })
}

export function useSetProtagonist(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: (id: number) => endpoints.setProtagonist(id),
    onSuccess: invalidate,
  })
}

export function useEntityHistory(entityId: number | undefined) {
  return useQuery({
    queryKey: ["canon", "entity-history", entityId ?? 0],
    queryFn: () => endpoints.entityHistory(entityId ?? 0),
    enabled: entityId !== undefined && entityId > 0,
  })
}

// ───────────────────────────── relations ─────────────────────────────

export function useRelations(bookId: number, entityId?: number) {
  return useQuery({
    queryKey: keys.canon.relations(bookId, entityId),
    queryFn: () => endpoints.listRelations(bookId, entityId),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

export function useCreateRelation(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: (input: RelationInput) =>
      endpoints.createRelation(bookId, input),
    onSuccess: invalidate,
  })
}

export function usePatchRelation(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: ({ id, patch }: { id: number; patch: RelationPatch }) =>
      endpoints.patchRelation(id, patch),
    onSuccess: invalidate,
  })
}

export function useDeleteRelation(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: (id: number) => endpoints.deleteRelation(id),
    onSuccess: invalidate,
  })
}

// ───────────────────────────── timeline ─────────────────────────────

export function useTimeline(bookId: number, trackId?: number) {
  return useQuery({
    queryKey: keys.canon.timeline(bookId, trackId),
    queryFn: () => endpoints.listTimeline(bookId, trackId),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

export function useCreateTimelineEvent(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: (input: TimelineEventInput) =>
      endpoints.createTimelineEvent(bookId, input),
    onSuccess: invalidate,
  })
}

export function usePatchTimelineEvent(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: ({ id, patch }: { id: number; patch: TimelineEventPatch }) =>
      endpoints.patchTimelineEvent(id, patch),
    onSuccess: invalidate,
  })
}

export function useDeleteTimelineEvent(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: (id: number) => endpoints.deleteTimelineEvent(id),
    onSuccess: invalidate,
  })
}

export function useMoveTimelineEvent(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: ({
      id,
      afterId,
      beforeId,
    }: {
      id: number
      afterId?: number
      beforeId?: number
    }) =>
      endpoints.moveTimelineEvent(id, {
        after_id: afterId,
        before_id: beforeId,
      }),
    onSuccess: invalidate,
  })
}

// ───────────────────────────── memory notes ─────────────────────────────

export function useMemoryNotes(
  bookId: number,
  params: { kind?: string; status?: string } = {}
) {
  const filters = [params.kind, params.status].join("|")
  return useQuery({
    queryKey: keys.canon.notes(bookId, filters),
    queryFn: () => endpoints.listMemoryNotes(bookId, params),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

export function useCreateMemoryNote(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: (input: MemoryNoteInput) =>
      endpoints.createMemoryNote(bookId, input),
    onSuccess: invalidate,
  })
}

export function usePatchMemoryNote(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: ({ id, patch }: { id: number; patch: MemoryNotePatch }) =>
      endpoints.patchMemoryNote(id, patch),
    onSuccess: invalidate,
  })
}

export function useDeleteMemoryNote(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: (id: number) => endpoints.deleteMemoryNote(id),
    onSuccess: invalidate,
  })
}

// ───────────────────────────── foreshadowing ─────────────────────────────

export function useForeshadowing(bookId: number, status?: string, trackId?: number) {
  const filter = `${status ?? "all"}|${trackId ?? "shared"}`
  return useQuery({
    queryKey: keys.canon.foreshadowing(bookId, filter),
    queryFn: () => endpoints.listForeshadowing(bookId, status, trackId),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

export function useCreateForeshadowing(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: (input: ForeshadowingInput) =>
      endpoints.createForeshadowing(bookId, input),
    onSuccess: invalidate,
  })
}

export function usePatchForeshadowing(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: ({ id, patch }: { id: number; patch: ForeshadowingPatch }) =>
      endpoints.patchForeshadowing(id, patch),
    onSuccess: invalidate,
  })
}

export function useDeleteForeshadowing(bookId: number) {
  const invalidate = useCanonInvalidator(bookId)
  return useMutation({
    mutationFn: (id: number) => endpoints.deleteForeshadowing(id),
    onSuccess: invalidate,
  })
}

// ───────────────────────────── memory snapshot ─────────────────────────────

export function useMemory(
  bookId: number,
  params: { trackId?: number; chapterId?: number } = {}
) {
  const position = `${params.trackId ?? "main"}:${params.chapterId ?? "next"}`
  return useQuery({
    queryKey: keys.canon.memory(bookId, position),
    queryFn: () => endpoints.getMemory(bookId, params),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

/** The retained rows of every canon system, shown by Book settings while a switch is off. */
export function useCanonCounts(bookId: number) {
  return useQuery({
    queryKey: keys.canon.counts(bookId),
    queryFn: () => endpoints.getCanonCounts(bookId),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

// ───────────────────────────── changes ─────────────────────────────

export function useChanges(bookId: number, params: ChangeQueryParams = {}) {
  const filter = [
    params.chapterId,
    params.sessionId,
    params.targetType,
    params.actor,
    params.cursor,
  ].join("|")
  return useQuery({
    queryKey: keys.changes.list(bookId, filter),
    queryFn: () => endpoints.listChanges(bookId, params),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

export function useRevertChange(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (changeId: number) => endpoints.revertChange(changeId, {}),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.changes.all })
      void queryClient.invalidateQueries({ queryKey: ["canon"] })
      void queryClient.invalidateQueries({
        queryKey: keys.books.overview(bookId),
      })
    },
  })
}
