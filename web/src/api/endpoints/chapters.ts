/** Chapter endpoints (spec/10 §2.2). */
import { api } from "@/api/client"
import type { ChapterContentPatch } from "@/api/gen/ChapterContentPatch"
import type { ChapterContentResult } from "@/api/gen/ChapterContentResult"
import type { ChapterContentTextPut } from "@/api/gen/ChapterContentTextPut"
import type { ApproveChapterResponse } from "@/api/gen/ApproveChapterResponse"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { ChapterPatch } from "@/api/gen/ChapterPatch"
import type { ChapterVersionDto } from "@/api/gen/ChapterVersionDto"
import type { ChapterVersionMetaDto } from "@/api/gen/ChapterVersionMetaDto"
import type { ChapterOutlineInput } from "@/api/gen/ChapterOutlineInput"
import type { ChapterWriteInput } from "@/api/gen/ChapterWriteInput"
import type { ChapterWriteResponse } from "@/api/gen/ChapterWriteResponse"
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

/** `POST /chapters/:id/outline` (spec/08 §1.1, D-146): start the planner session. */
export function writeOutline(
  chapterId: number,
  input: ChapterOutlineInput,
): Promise<ChapterWriteResponse> {
  return api.post<ChapterWriteResponse>(`/chapters/${chapterId}/outline`, input)
}

/** `POST /chapters/:id/write` (spec/08 §1.1): start the writer session. */
export function writeChapter(
  chapterId: number,
  input: ChapterWriteInput,
): Promise<ChapterWriteResponse> {
  return api.post<ChapterWriteResponse>(`/chapters/${chapterId}/write`, input)
}

export function submitChapter(chapterId: number): Promise<ChapterDto> {
  return api.post<ChapterDto>(`/chapters/${chapterId}/submit`)
}

export function approveChapter(chapterId: number): Promise<ApproveChapterResponse> {
  return api.post<ApproveChapterResponse>(`/chapters/${chapterId}/approve`)
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
