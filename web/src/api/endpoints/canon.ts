/** Canon endpoints (spec/10 §2.4). */
import { api } from "@/api/client"
import type { AppearanceDto } from "@/api/gen/AppearanceDto"
import type { AppearanceInput } from "@/api/gen/AppearanceInput"
import type { ChangeDto } from "@/api/gen/ChangeDto"
import type { ChangePageDto } from "@/api/gen/ChangePageDto"
import type { EntityDto } from "@/api/gen/EntityDto"
import type { EntityInput } from "@/api/gen/EntityInput"
import type { EntityListItemDto } from "@/api/gen/EntityListItemDto"
import type { EntityMergeInput } from "@/api/gen/EntityMergeInput"
import type { EntityPatch } from "@/api/gen/EntityPatch"
import type { ForeshadowingDto } from "@/api/gen/ForeshadowingDto"
import type { ForeshadowingInput } from "@/api/gen/ForeshadowingInput"
import type { ForeshadowingPatch } from "@/api/gen/ForeshadowingPatch"
import type { MemoryNoteDto } from "@/api/gen/MemoryNoteDto"
import type { MemoryNoteInput } from "@/api/gen/MemoryNoteInput"
import type { MemoryNotePatch } from "@/api/gen/MemoryNotePatch"
import type { MemoryViewDto } from "@/api/gen/MemoryViewDto"
import type { RelationDto } from "@/api/gen/RelationDto"
import type { RelationInput } from "@/api/gen/RelationInput"
import type { RelationPatch } from "@/api/gen/RelationPatch"
import type { RevertInput } from "@/api/gen/RevertInput"
import type { TimelineEventDto } from "@/api/gen/TimelineEventDto"
import type { TimelineEventInput } from "@/api/gen/TimelineEventInput"
import type { TimelineEventPatch } from "@/api/gen/TimelineEventPatch"
import type { TimelineMoveInput } from "@/api/gen/TimelineMoveInput"

// ───────────────────────────── entities ─────────────────────────────

export function listEntities(
  bookId: number,
  params: { kind?: string; status?: string; q?: string; trackId?: number } = {},
): Promise<EntityListItemDto[]> {
  const query = new URLSearchParams()
  if (params.kind !== undefined) query.set("kind", params.kind)
  if (params.status !== undefined) query.set("status", params.status)
  if (params.q !== undefined && params.q !== "") query.set("q", params.q)
  if (params.trackId !== undefined) query.set("track_id", String(params.trackId))
  const suffix = query.toString()
  return api.get<EntityListItemDto[]>(
    `/books/${bookId}/entities${suffix === "" ? "" : `?${suffix}`}`,
  )
}

export function createEntity(bookId: number, input: EntityInput): Promise<EntityDto> {
  return api.post<EntityDto>(`/books/${bookId}/entities`, input)
}

export function getEntity(entityId: number): Promise<EntityDto> {
  return api.get<EntityDto>(`/entities/${entityId}`)
}

export function patchEntity(entityId: number, patch: EntityPatch): Promise<EntityDto> {
  return api.patch<EntityDto>(`/entities/${entityId}`, patch)
}

export function deleteEntity(entityId: number): Promise<void> {
  return api.delete<void>(`/entities/${entityId}`)
}

export function mergeEntity(entityId: number, input: EntityMergeInput): Promise<EntityDto> {
  return api.post<EntityDto>(`/entities/${entityId}/merge`, input)
}

export function setProtagonist(entityId: number): Promise<EntityDto> {
  return api.post<EntityDto>(`/entities/${entityId}/protagonist`)
}

export function entityHistory(entityId: number): Promise<ChangeDto[]> {
  return api.get<ChangeDto[]>(`/entities/${entityId}/history`)
}

// ───────────────────────────── relations ─────────────────────────────

export function listRelations(
  bookId: number,
  entityId?: number,
): Promise<RelationDto[]> {
  const suffix = entityId === undefined ? "" : `?entity_id=${entityId}`
  return api.get<RelationDto[]>(`/books/${bookId}/relations${suffix}`)
}

export function createRelation(bookId: number, input: RelationInput): Promise<RelationDto> {
  return api.post<RelationDto>(`/books/${bookId}/relations`, input)
}

export function patchRelation(
  relationId: number,
  patch: RelationPatch,
): Promise<RelationDto> {
  return api.patch<RelationDto>(`/relations/${relationId}`, patch)
}

export function deleteRelation(relationId: number): Promise<void> {
  return api.delete<void>(`/relations/${relationId}`)
}

// ───────────────────────────── appearances ─────────────────────────────

export function chapterAppearances(chapterId: number): Promise<AppearanceDto[]> {
  return api.get<AppearanceDto[]>(`/chapters/${chapterId}/appearances`)
}

export function putAppearances(
  chapterId: number,
  appearances: AppearanceInput[],
): Promise<AppearanceDto[]> {
  return api.put<AppearanceDto[]>(`/chapters/${chapterId}/appearances`, appearances)
}

// ───────────────────────────── timeline ─────────────────────────────

export function listTimeline(bookId: number): Promise<TimelineEventDto[]> {
  return api.get<TimelineEventDto[]>(`/books/${bookId}/timeline`)
}

export function createTimelineEvent(
  bookId: number,
  input: TimelineEventInput,
): Promise<TimelineEventDto> {
  return api.post<TimelineEventDto>(`/books/${bookId}/timeline`, input)
}

export function patchTimelineEvent(
  eventId: number,
  patch: TimelineEventPatch,
): Promise<TimelineEventDto> {
  return api.patch<TimelineEventDto>(`/timeline-events/${eventId}`, patch)
}

export function deleteTimelineEvent(eventId: number): Promise<void> {
  return api.delete<void>(`/timeline-events/${eventId}`)
}

export function moveTimelineEvent(
  eventId: number,
  input: TimelineMoveInput,
): Promise<TimelineEventDto> {
  return api.post<TimelineEventDto>(`/timeline-events/${eventId}/move`, input)
}

// ───────────────────────────── memory notes ─────────────────────────────

export function listMemoryNotes(
  bookId: number,
  params: { kind?: string; status?: string } = {},
): Promise<MemoryNoteDto[]> {
  const query = new URLSearchParams()
  if (params.kind !== undefined) query.set("kind", params.kind)
  if (params.status !== undefined) query.set("status", params.status)
  const suffix = query.toString()
  return api.get<MemoryNoteDto[]>(
    `/books/${bookId}/memory-notes${suffix === "" ? "" : `?${suffix}`}`,
  )
}

export function createMemoryNote(
  bookId: number,
  input: MemoryNoteInput,
): Promise<MemoryNoteDto> {
  return api.post<MemoryNoteDto>(`/books/${bookId}/memory-notes`, input)
}

export function patchMemoryNote(
  noteId: number,
  patch: MemoryNotePatch,
): Promise<MemoryNoteDto> {
  return api.patch<MemoryNoteDto>(`/memory-notes/${noteId}`, patch)
}

export function deleteMemoryNote(noteId: number): Promise<void> {
  return api.delete<void>(`/memory-notes/${noteId}`)
}

// ───────────────────────────── foreshadowing ─────────────────────────────

export function listForeshadowing(
  bookId: number,
  status?: string,
): Promise<ForeshadowingDto[]> {
  const suffix = status === undefined ? "" : `?status=${status}`
  return api.get<ForeshadowingDto[]>(`/books/${bookId}/foreshadowing${suffix}`)
}

export function createForeshadowing(
  bookId: number,
  input: ForeshadowingInput,
): Promise<ForeshadowingDto> {
  return api.post<ForeshadowingDto>(`/books/${bookId}/foreshadowing`, input)
}

export function patchForeshadowing(
  itemId: number,
  patch: ForeshadowingPatch,
): Promise<ForeshadowingDto> {
  return api.patch<ForeshadowingDto>(`/foreshadowing/${itemId}`, patch)
}

export function deleteForeshadowing(itemId: number): Promise<void> {
  return api.delete<void>(`/foreshadowing/${itemId}`)
}

// ───────────────────────────── memory snapshot ─────────────────────────────

export function getMemory(
  bookId: number,
  params: { trackId?: number; chapterId?: number } = {},
): Promise<MemoryViewDto> {
  const query = new URLSearchParams()
  if (params.trackId !== undefined) query.set("track_id", String(params.trackId))
  if (params.chapterId !== undefined) query.set("chapter_id", String(params.chapterId))
  const suffix = query.toString()
  return api.get<MemoryViewDto>(
    `/books/${bookId}/memory${suffix === "" ? "" : `?${suffix}`}`,
  )
}

// ───────────────────────────── changes ─────────────────────────────

export type ChangeQueryParams = {
  chapterId?: number
  sessionId?: number
  targetType?: string
  actor?: string
  cursor?: string
  limit?: number
}

export function listChanges(bookId: number, params: ChangeQueryParams = {}): Promise<ChangePageDto> {
  const query = new URLSearchParams()
  if (params.chapterId !== undefined) query.set("chapter_id", String(params.chapterId))
  if (params.sessionId !== undefined) query.set("session_id", String(params.sessionId))
  if (params.targetType !== undefined) query.set("target_type", params.targetType)
  if (params.actor !== undefined) query.set("actor", params.actor)
  if (params.cursor !== undefined) query.set("cursor", params.cursor)
  if (params.limit !== undefined) query.set("limit", String(params.limit))
  const suffix = query.toString()
  return api.get<ChangePageDto>(`/books/${bookId}/changes${suffix === "" ? "" : `?${suffix}`}`)
}

export function revertChange(changeId: number, input: RevertInput = {}): Promise<ChangeDto> {
  return api.post<ChangeDto>(`/changes/${changeId}/revert`, input)
}
