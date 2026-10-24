/** Track and chapter-list queries and mutations (spec/11 §4.1). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/tracks"
import { keys } from "@/api/keys"
import type { ChapterInput } from "@/api/gen/ChapterInput"
import type { TrackPatch } from "@/api/gen/TrackPatch"

export function useTracks(bookId: number) {
  return useQuery({
    queryKey: keys.books.tracks(bookId),
    queryFn: () => endpoints.listTracks(bookId),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

export function usePatchTrack(bookId: number, trackId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (patch: TrackPatch) => endpoints.patchTrack(trackId, patch),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.books.tracks(bookId) })
    },
  })
}

export function useChapters(trackId: number) {
  return useQuery({
    queryKey: keys.tracks.chapters(trackId),
    queryFn: () => endpoints.listChapters(trackId),
    enabled: Number.isFinite(trackId) && trackId > 0,
  })
}

export function useCreateChapter(trackId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (input: ChapterInput) => endpoints.createChapter(trackId, input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.tracks.chapters(trackId) })
      void queryClient.invalidateQueries({ queryKey: keys.books.all })
    },
  })
}

export function useReorderChapters(bookId: number, trackId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (ids: number[]) => endpoints.reorderChapters(trackId, ids),
    onSuccess: (list) => {
      queryClient.setQueryData(keys.tracks.chapters(trackId), list)
      void queryClient.invalidateQueries({ queryKey: keys.books.detail(bookId) })
    },
  })
}
