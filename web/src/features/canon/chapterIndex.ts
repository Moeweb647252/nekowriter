/**
 * Chapter numbers behind the canon pages' chapter references (spec/10 §2.4, D-054).
 *
 * The canon DTOs carry a chapter **id** (a reference), not its number: a chapter's number is its
 * `ord` inside its track, and ids are global row ids. Rendering the id as "Chapter <id>" is wrong
 * as soon as a book is not the first one in the database or a side track exists, so the canon
 * pages resolve the id against the book's chapter lists first.
 */
import { useMemo } from "react"
import { useQueries } from "@tanstack/react-query"

import { listChapters } from "@/api/endpoints/tracks"
import { keys } from "@/api/keys"
import type { TrackDto } from "@/api/gen/TrackDto"

/** One chapter as the canon pages need it: its number, title and track. */
export type ChapterEntry = {
  id: number
  number: number
  title: string
  trackId: number
  trackKind: TrackDto["kind"]
  trackTitle: string
}

/** Chapter id → chapter entry for every track of the book. */
export type ChapterIndex = Map<number, ChapterEntry>

/** Loads the chapter list of every track of the book and indexes it by chapter id. */
export function useChapterIndex(tracks: TrackDto[] | undefined): ChapterIndex {
  const queries = useQueries({
    queries: (tracks ?? []).map((track) => ({
      queryKey: keys.tracks.chapters(track.id),
      queryFn: () => listChapters(track.id),
      enabled: track.id > 0,
    })),
  })
  return useMemo(() => {
    const index: ChapterIndex = new Map()
    for (const [position, query] of queries.entries()) {
      const track = tracks?.[position]
      if (track === undefined || query.data === undefined) {
        continue
      }
      for (const chapter of query.data) {
        index.set(chapter.id, {
          id: chapter.id,
          number: chapter.number,
          title: chapter.title,
          trackId: track.id,
          trackKind: track.kind,
          trackTitle: track.title,
        })
      }
    }
    return index
  }, [queries, tracks])
}

/** The `t` shape the label helper needs (i18next's `TFunction` is assignable to it). */
export type Translate = (key: string, options?: Record<string, unknown>) => string

/**
 * Renders a chapter id as its label: `第12章` for the main track and `番外《轨道名》第2章` for a
 * side track (spec/06 §2, spec/05 §1.5). A reference whose chapter row is gone falls back to the
 * id, so a change log entry never shows a made-up number.
 */
export function chapterRefLabel(
  t: Translate,
  index: ChapterIndex,
  chapterId: number | null | undefined,
): string {
  if (chapterId === null || chapterId === undefined) {
    return ""
  }
  const entry = index.get(chapterId)
  if (entry === undefined) {
    return t("canon:chapter_unknown", { id: chapterId })
  }
  return entry.trackKind === "main"
    ? t("canon:chapter_number", { number: entry.number })
    : t("canon:chapter_number_side", {
        number: entry.number,
        track: entry.trackTitle,
      })
}
