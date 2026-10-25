/** Track and chapter-list queries and mutations (spec/11 §4.1). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/tracks"
import { keys } from "@/api/keys"
import type { ChapterInput } from "@/api/gen/ChapterInput"
import type { TrackInput } from "@/api/gen/TrackInput"
import type { TrackPatch } from "@/api/gen/TrackPatch"

export function useTracks(bookId: number) {
  return useQuery({
    queryKey: keys.books.tracks(bookId),
    queryFn: () => endpoints.listTracks(bookId),
    enabled: Number.isFinite(bookId) && bookId > 0,
  })
}

/** One track, for the side-story page (spec/11 §3 `tracks/:trackId`). */
export function useTrack(trackId: number) {
  return useQuery({
    queryKey: keys.tracks.detail(trackId),
    queryFn: () => endpoints.getTrack(trackId),
    enabled: Number.isFinite(trackId) && trackId > 0,
  })
}

/** Creates a side story and refreshes the navigator (spec/08 §6). */
export function useCreateTrack(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (input: TrackInput) => endpoints.createTrack(bookId, input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.books.tracks(bookId) })
      void queryClient.invalidateQueries({ queryKey: keys.books.overview(bookId) })
      void queryClient.invalidateQueries({ queryKey: keys.books.detail(bookId) })
    },
  })
}

export function usePatchTrack(bookId: number, trackId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (patch: TrackPatch) => endpoints.patchTrack(trackId, patch),
    onSuccess: (track) => {
      queryClient.setQueryData(keys.tracks.detail(trackId), track)
      void queryClient.invalidateQueries({ queryKey: keys.books.tracks(bookId) })
    },
  })
}

/** Deletes a side story; its chapters cascade (spec/08 §1.3). */
export function useDeleteTrack(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (trackId: number) => endpoints.deleteTrack(trackId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.books.tracks(bookId) })
      void queryClient.invalidateQueries({ queryKey: keys.books.overview(bookId) })
      void queryClient.invalidateQueries({ queryKey: keys.books.detail(bookId) })
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
