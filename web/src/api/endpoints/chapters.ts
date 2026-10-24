/** Chapter endpoints (spec/10 §2.2). */
import { api } from "@/api/client"
import type { ChapterContentPatch } from "@/api/gen/ChapterContentPatch"
import type { ChapterContentResult } from "@/api/gen/ChapterContentResult"
import type { ChapterContentTextPut } from "@/api/gen/ChapterContentTextPut"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { ChapterPatch } from "@/api/gen/ChapterPatch"
import type { ChapterVersionDto } from "@/api/gen/ChapterVersionDto"
import type { ChapterVersionMetaDto } from "@/api/gen/ChapterVersionMetaDto"
import type { ManualVersionInput } from "@/api/gen/ManualVersionInput"

export function getChapter(chapterId: number): Promise<ChapterDto> {
  return api.get<ChapterDto>(`/chapters/${chapterId}`)
}

export function patchChapter(chapterId: number, patch: ChapterPatch): Promise<ChapterDto> {
  return api.patch<ChapterDto>(`/chapters/${chapterId}`, patch)
}

export function deleteChapter(chapterId: number): Promise<void> {
  return api.delete<void>(`/chapters/${chapterId}`)
}

export function patchContent(
  chapterId: number,
  patch: ChapterContentPatch,
): Promise<ChapterContentResult> {
  return api.patch<ChapterContentResult>(`/chapters/${chapterId}/content`, patch)
}

export function putContentText(
  chapterId: number,
  put: ChapterContentTextPut,
): Promise<ChapterContentResult> {
  return api.put<ChapterContentResult>(`/chapters/${chapterId}/content/text`, put)
}

export function submitChapter(chapterId: number): Promise<ChapterDto> {
  return api.post<ChapterDto>(`/chapters/${chapterId}/submit`)
}

export function approveChapter(chapterId: number): Promise<ChapterDto> {
  return api.post<ChapterDto>(`/chapters/${chapterId}/approve`)
}

export function listVersions(chapterId: number): Promise<ChapterVersionMetaDto[]> {
  return api.get<ChapterVersionMetaDto[]>(`/chapters/${chapterId}/versions`)
}

export function saveVersion(
  chapterId: number,
  input: ManualVersionInput,
): Promise<ChapterVersionMetaDto> {
  return api.post<ChapterVersionMetaDto>(`/chapters/${chapterId}/versions`, input)
}

export function getVersion(versionId: number): Promise<ChapterVersionDto> {
  return api.get<ChapterVersionDto>(`/chapter-versions/${versionId}`)
}

export function restoreVersion(versionId: number): Promise<ChapterDto> {
  return api.post<ChapterDto>(`/chapter-versions/${versionId}/restore`)
}
