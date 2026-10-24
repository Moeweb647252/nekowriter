/** Track and chapter-list endpoints (spec/10 §2.2). */
import { api } from "@/api/client"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { ChapterInput } from "@/api/gen/ChapterInput"
import type { ChapterListItemDto } from "@/api/gen/ChapterListItemDto"
import type { TrackDto } from "@/api/gen/TrackDto"
import type { TrackPatch } from "@/api/gen/TrackPatch"

export function listTracks(bookId: number): Promise<TrackDto[]> {
  return api.get<TrackDto[]>(`/books/${bookId}/tracks`)
}

export function patchTrack(trackId: number, patch: TrackPatch): Promise<TrackDto> {
  return api.patch<TrackDto>(`/tracks/${trackId}`, patch)
}

export function listChapters(trackId: number): Promise<ChapterListItemDto[]> {
  return api.get<ChapterListItemDto[]>(`/tracks/${trackId}/chapters`)
}

export function createChapter(trackId: number, input: ChapterInput): Promise<ChapterDto> {
  return api.post<ChapterDto>(`/tracks/${trackId}/chapters`, input)
}

export function reorderChapters(
  trackId: number,
  ids: number[],
): Promise<ChapterListItemDto[]> {
  return api.post<ChapterListItemDto[]>(`/tracks/${trackId}/chapters/reorder`, { ids })
}
