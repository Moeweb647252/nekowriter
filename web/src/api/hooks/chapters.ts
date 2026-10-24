/** Chapter queries and mutations (spec/11 §4.1). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/chapters"
import { keys } from "@/api/keys"
import type { ChapterContentPatch } from "@/api/gen/ChapterContentPatch"
import type { ChapterContentResult } from "@/api/gen/ChapterContentResult"
import type { ChapterContentTextPut } from "@/api/gen/ChapterContentTextPut"
import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { ChapterPatch } from "@/api/gen/ChapterPatch"
import type { ManualVersionInput } from "@/api/gen/ManualVersionInput"
import { useChapterDoc } from "@/stores/chapterDoc"

export function useChapter(chapterId: number) {
  return useQuery({
    queryKey: keys.chapters.detail(chapterId),
    queryFn: () => endpoints.getChapter(chapterId),
    enabled: Number.isFinite(chapterId) && chapterId > 0,
  })
}

export function usePatchChapter(chapterId: number, bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (patch: ChapterPatch) => endpoints.patchChapter(chapterId, patch),
    onSuccess: (chapter) => {
      queryClient.setQueryData(keys.chapters.detail(chapterId), chapter)
      void queryClient.invalidateQueries({ queryKey: keys.tracks.chapters(chapter.track_id) })
      void queryClient.invalidateQueries({ queryKey: keys.books.overview(bookId) })
    },
  })
}

export function useSubmitChapter(chapterId: number, bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => endpoints.submitChapter(chapterId),
    onSuccess: (chapter) => {
      queryClient.setQueryData(keys.chapters.detail(chapterId), chapter)
      void queryClient.invalidateQueries({ queryKey: keys.tracks.chapters(chapter.track_id) })
      void queryClient.invalidateQueries({ queryKey: keys.books.overview(bookId) })
    },
  })
}

export function useApproveChapter(chapterId: number, bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => endpoints.approveChapter(chapterId),
    onSuccess: (chapter) => {
      queryClient.setQueryData(keys.chapters.detail(chapterId), chapter)
      void queryClient.invalidateQueries({ queryKey: keys.tracks.chapters(chapter.track_id) })
      void queryClient.invalidateQueries({ queryKey: keys.books.overview(bookId) })
    },
  })
}

/**
 * Content edits: the store applies its own response so the open chapter never waits for SSE
 * (spec/11 §4.3), and the chapter query drops its stale content.
 */
export function useContentMutations(chapterId: number, bookId: number) {
  const queryClient = useQueryClient()
  const applyOps = useChapterDoc((state) => state.applyOps)

  const settle = (result: ChapterContentResult) => {
    if (applyOps(result.version, result.ops, "user") === "gap") {
      // A gap means this tab missed events; refetching is the only correct recovery.
      void queryClient.invalidateQueries({ queryKey: keys.chapters.detail(chapterId) })
    }
    void queryClient.invalidateQueries({ queryKey: keys.chapters.detail(chapterId) })
    void queryClient.invalidateQueries({ queryKey: keys.books.overview(bookId) })
  }

  const patch = useMutation({
    mutationFn: (body: ChapterContentPatch) => endpoints.patchContent(chapterId, body),
    onSuccess: settle,
  })
  const putText = useMutation({
    mutationFn: (body: ChapterContentTextPut) => endpoints.putContentText(chapterId, body),
    onSuccess: settle,
  })
  return { patch, putText }
}

export function useVersions(chapterId: number) {
  return useQuery({
    queryKey: keys.chapters.versions(chapterId),
    queryFn: () => endpoints.listVersions(chapterId),
    enabled: Number.isFinite(chapterId) && chapterId > 0,
  })
}

export function useSaveVersion(chapterId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (input: ManualVersionInput) => endpoints.saveVersion(chapterId, input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.chapters.versions(chapterId) })
    },
  })
}

export function useVersion(versionId: number | null) {
  return useQuery({
    queryKey: keys.chapters.version(versionId ?? 0),
    queryFn: () => endpoints.getVersion(versionId ?? 0),
    enabled: versionId !== null && versionId > 0,
  })
}

export function useRestoreVersion(bookId: number) {
  const queryClient = useQueryClient()
  const load = useChapterDoc((state) => state.load)
  return useMutation({
    mutationFn: (versionId: number) => endpoints.restoreVersion(versionId),
    onSuccess: (chapter: ChapterDto) => {
      load(chapter.id, chapter.content_version, chapter.content)
      queryClient.setQueryData(keys.chapters.detail(chapter.id), chapter)
      void queryClient.invalidateQueries({ queryKey: keys.chapters.versions(chapter.id) })
      void queryClient.invalidateQueries({ queryKey: keys.tracks.chapters(chapter.track_id) })
      void queryClient.invalidateQueries({ queryKey: keys.books.overview(bookId) })
    },
  })
}
