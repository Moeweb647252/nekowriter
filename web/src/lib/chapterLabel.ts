/**
 * Chapter labels (spec/11 §6): "Chapter {{number}}" on the main track, and
 * "{{track}} · Chapter {{number}}" on a side track.
 *
 * The API sends only data (`number`, the track's `kind` and `title`, spec/10 §2.8), so the label is
 * composed here — one helper for every screen that mentions a chapter out of its own track
 * context (the overview's attention list, the next-action card).
 */
import type { TFunction } from "i18next"

/** The track fields a chapter label needs (a `TrackProgressDto` satisfies this). */
export type ChapterLabelTrack = {
  kind: string
  /** Empty for the main track; the UI localizes it. */
  title: string
}

/** Builds the localized label of a chapter, using the side-track form when it belongs to one. */
export function formatChapterLabel(
  t: TFunction,
  chapter: { number: number; track?: ChapterLabelTrack | null },
): string {
  const track = chapter.track
  if (track !== undefined && track !== null && track.kind !== "main") {
    return t("chapter:side_label", { track: track.title, number: chapter.number })
  }
  return t("chapter:label", { number: chapter.number })
}

/** The localized track name: the main story has no stored title (spec/02 §2). */
export function trackName(t: TFunction, track: ChapterLabelTrack): string {
  return track.kind === "main" ? t("book:main_story") : track.title
}
