/** The Memory page: a context selector and the three columns of tiered memory, rendered exactly
 * as the writer receives them, plus the facts and world rules table (PROJECT §8.8, spec/06 §4). */
import { useState } from "react"
import { useParams } from "react-router"
import { useTranslation } from "react-i18next"

import { useChapters, useTracks } from "@/api/hooks/tracks"
import { useMemory } from "@/api/hooks/useCanon"
import { NotesTable } from "@/features/canon/NotesPage"
import { EmptyState } from "@/components/common/EmptyState"
import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"

export default function MemoryPage() {
  const { t } = useTranslation()
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const tracks = useTracks(bookId)
  const main = tracks.data?.find((track) => track.kind === "main")
  // The position selector: the main story by default, or a side story. A side track's memory is
  // built from its own chapters topped up from the main story up to its anchor (spec/06 §4.1).
  const [trackId, setTrackId] = useState<number | undefined>(undefined)
  const sideTracks = (tracks.data ?? []).filter((track) => track.kind === "side")
  const chapters = useChapters(trackId ?? main?.id ?? 0)

  // "Next chapter of …" or a specific chapter (PROJECT §8.8).
  const [chapterId, setChapterId] = useState<number | undefined>(undefined)
  const memory = useMemory(bookId, { trackId, chapterId })

  if (memory.isPending) {
    return <Skeleton className="m-6 h-64" />
  }
  if (memory.isError || memory.data === undefined) {
    return (
      <div className="p-6">
        <EmptyState title={t("common:error_title")} description={t("common:error_body")} />
      </div>
    )
  }
  const snapshot = memory.data.snapshot
  const systems = memory.data.systems
  const allSystemsOff =
    !systems.entities && !systems.timeline && !systems.memory && !systems.foreshadowing
  const countFragments: string[] = []
  if (systems.entities) {
    countFragments.push(
      t("canon:memory_counts.entities", {
        character: snapshot.counts.character,
        organization: snapshot.counts.organization,
        location: snapshot.counts.location,
        item: snapshot.counts.item,
        other: snapshot.counts.other,
      }),
    )
  }
  if (systems.timeline) {
    countFragments.push(t("canon:memory_counts.timeline", { event: snapshot.counts.event }))
  }
  if (systems.memory) {
    countFragments.push(
      t("canon:memory_counts.memory", {
        fact: snapshot.counts.fact,
        world_rule: snapshot.counts.world_rule,
      }),
    )
  }
  if (systems.foreshadowing) {
    countFragments.push(
      t("canon:memory_counts.foreshadowing", {
        foreshadowing_open: snapshot.counts.foreshadowing_open,
      }),
    )
  }

  return (
    <div className="mx-auto max-w-6xl space-y-4 p-6">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold">{t("canon:memory")}</h1>
          <p className="text-xs text-muted-foreground">{t("canon:memory_hint")}</p>
        </div>
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <label className="flex items-center gap-2">
            {t("canon:memory_track")}
            <select
              value={trackId ?? ""}
              onChange={(event) => {
                setTrackId(
                  event.target.value === "" ? undefined : Number(event.target.value),
                )
                // The chapter belongs to the track; a stale id would 400.
                setChapterId(undefined)
              }}
              className="h-8 rounded-md border bg-background px-2 text-xs text-foreground"
            >
              <option value="">{t("book:main_story")}</option>
              {sideTracks.map((track) => (
                <option key={track.id} value={track.id}>
                  {track.title}
                </option>
              ))}
            </select>
          </label>
          <label className="flex items-center gap-2">
            {t("canon:memory_context")}
            <select
              value={chapterId ?? ""}
              onChange={(event) =>
                setChapterId(event.target.value === "" ? undefined : Number(event.target.value))
              }
              className="h-8 rounded-md border bg-background px-2 text-xs text-foreground"
            >
              <option value="">{t("canon:memory_next_chapter")}</option>
              {(chapters.data ?? []).map((chapter) => (
                <option key={chapter.id} value={chapter.id}>
                  {t("canon:chapter_number_title", {
                    number: chapter.number,
                    title: chapter.title,
                  })}
                </option>
              ))}
            </select>
          </label>
        </div>
      </header>

      {snapshot.anchor === null ? null : (
        <p className="rounded-md border border-dashed px-3 py-2 text-xs text-muted-foreground">
          {t(`canon:anchor_notice.${snapshot.anchor.kind}`, {
            label: snapshot.anchor.label,
          })}
        </p>
      )}

      <div className="space-y-1 rounded-md border border-dashed px-3 py-2 text-xs text-muted-foreground">
        <p>{t("canon:memory_variant")}</p>
        {memory.data.recent_chapters.length === 0 ? null : (
          <div>
            <p className="font-medium">{t("canon:memory_recent_chapters")}</p>
            <ul className="list-inside list-disc">
              {memory.data.recent_chapters.map((chapter) => (
                <li key={chapter.chapter_id}>
                  {t("canon:memory_recent_chapter_item", {
                    label: chapter.label,
                    title: chapter.title,
                    characters: chapter.characters,
                    partial: chapter.partial ? t("canon:memory_recent_partial") : "",
                  })}
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>

      {allSystemsOff ? (
        <EmptyState
          title={t("canon:memory_all_off_title")}
          description={t("canon:memory_all_off_hint")}
        />
      ) : (
      <div className="grid gap-3 md:grid-cols-3">
        <MemoryColumn title={t("canon:memory_short_term")}>
          {snapshot.is_first ? (
            <p className="text-xs text-muted-foreground">{t("canon:memory_first_chapter")}</p>
          ) : (
            <>
              <ul className="space-y-1 text-sm">
                {snapshot.recent_summaries.map((summary) => (
                  <li key={summary.chapter_id}>
                    <span className="font-medium">
                      {summary.label}
                      {summary.title === "" ? "" : `《${summary.title}》`}
                    </span>
                    <p className="text-xs text-muted-foreground">
                      {summary.summary === "" ? t("canon:memory_no_summary") : summary.summary}
                    </p>
                  </li>
                ))}
              </ul>
              {snapshot.previous_tail === null ? null : (
                <div className="mt-2">
                  <p className="text-[0.65rem] text-muted-foreground">
                    {t("canon:memory_previous_tail", { label: snapshot.previous_tail.label })}
                  </p>
                  <p className="text-xs whitespace-pre-wrap">{snapshot.previous_tail.text}</p>
                </div>
              )}
              {snapshot.protagonist === null ? null : (
                <div className="mt-2 rounded-md border border-primary/30 bg-primary/5 p-2">
                  <p className="text-xs font-medium">
                    {snapshot.protagonist.name}
                    {snapshot.protagonist.status === "active"
                      ? ""
                      : ` · ${t(`canon:status.${snapshot.protagonist.status}`)}`}
                    {snapshot.protagonist.after_anchor ? (
                      <span className="ml-1 rounded bg-muted px-1 text-[0.6rem] font-normal text-muted-foreground">
                        {t("canon:after_anchor")}
                      </span>
                    ) : null}
                  </p>
                  <p className="text-xs whitespace-pre-wrap">{snapshot.protagonist.state}</p>
                  {Object.entries(snapshot.protagonist.attributes).length === 0 ? null : (
                    <p className="mt-1 text-[0.65rem] text-muted-foreground">
                      {Object.entries(snapshot.protagonist.attributes)
                        .map(([key, value]) => `${key}：${value}`)
                        .join("；")}
                    </p>
                  )}
                </div>
              )}
            </>
          )}
        </MemoryColumn>

        {systems.entities ? (
        <MemoryColumn title={t("canon:memory_mid_term", { count: snapshot.window_mid })}>
          <p className="text-[0.65rem] text-muted-foreground">
            {t("canon:memory_recent_changes")}
          </p>
          {snapshot.recent_changes.length === 0 ? (
            <p className="text-xs text-muted-foreground">{t("canon:memory_no_changes")}</p>
          ) : (
            <ul className="space-y-0.5 text-xs">
              {snapshot.recent_changes.map((change, index) => (
                <li key={index}>
                  {change.chapter_label} · {change.target}：{change.note}
                </li>
              ))}
            </ul>
          )}
          <p className="mt-2 text-[0.65rem] text-muted-foreground">
            {t("canon:memory_appearances")}
          </p>
          {snapshot.appearances.length === 0 ? (
            <p className="text-xs text-muted-foreground">{t("canon:memory_no_appearances")}</p>
          ) : (
            <ul className="space-y-0.5 text-xs">
              {snapshot.appearances.map((item) => (
                <li key={item.id}>
                  {item.name}(#{item.id})：{t("canon:memory_times", { count: item.count })}
                </li>
              ))}
            </ul>
          )}
          {snapshot.absent_important.length === 0 ? null : (
            <>
              <p className="mt-2 text-[0.65rem] text-muted-foreground">
                {t("canon:memory_absent")}
              </p>
              <ul className="space-y-0.5 text-xs">
                {snapshot.absent_important.map((item) => (
                  <li key={item.id}>
                    {item.name}(#{item.id})：{t("canon:memory_last_seen", { label: item.last_label })}
                  </li>
                ))}
              </ul>
            </>
          )}
        </MemoryColumn>
        ) : null}

        <MemoryColumn title={t("canon:memory_long_term")}>
          {systems.memory && snapshot.key_notes.length !== 0 ? (
            <>
              <p className="text-[0.65rem] text-muted-foreground">
                {t("canon:memory_key_notes")}
              </p>
              <ul className="space-y-0.5 text-xs">
                {snapshot.key_notes.map((note) => (
                  <li key={note.id}>
                    [{t(`canon:note_kind.${note.kind}`)}#{note.id}] {note.content}
                  </li>
                ))}
              </ul>
            </>
          ) : null}
          {!systems.foreshadowing || snapshot.open_foreshadowing.length === 0 ? null : (
            <>
              <p className="mt-2 text-[0.65rem] text-muted-foreground">
                {t("canon:memory_open_foreshadowing")}
              </p>
              <ul className="space-y-0.5 text-xs">
                {snapshot.open_foreshadowing.map((item) => (
                  <li key={item.id}>
                    #{item.id} {item.title}
                    {item.planted_label === ""
                      ? ""
                      : `（${t("canon:memory_planted", { label: item.planted_label })}）`}
                  </li>
                ))}
              </ul>
            </>
          )}
          {systems.entities ? (
            <>
              <p className="mt-2 text-[0.65rem] text-muted-foreground">
                {t("canon:memory_cast")}
              </p>
              <ul className="space-y-0.5 text-xs">
                {snapshot.cast.map((item) => (
                  <li key={item.id}>
                    #{item.id} {item.name}【{t(`canon:kind.${item.kind}`)}】
                    {item.summary === "" ? "" : ` — ${item.summary}`}
                  </li>
                ))}
              </ul>
            </>
          ) : null}
          {countFragments.length === 0 ? null : (
            <p className="mt-2 text-[0.65rem] text-muted-foreground">
              {t("canon:memory_counts_label")}{" "}
              {countFragments.join(t("canon:memory_counts_separator"))}
            </p>
          )}
        </MemoryColumn>
      </div>
      )}

      <section>
        <h2 className="mb-1 text-sm font-medium">{t("canon:notes")}</h2>
        <p className="mb-2 text-xs text-muted-foreground">{t("canon:notes_hint")}</p>
        <NotesTable bookId={bookId} />
      </section>

      <section>
        <h2 className="mb-2 text-sm font-medium">{t("canon:memory_rendered")}</h2>
        <pre className="max-h-[28rem] overflow-auto rounded-md border bg-muted/40 p-3 text-xs whitespace-pre-wrap">
          {memory.data.rendered}
        </pre>
      </section>
    </div>
  )
}

/** One memory column. */
function MemoryColumn({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className={cn("min-h-32 rounded-md border p-3")}>
      <h2 className="mb-2 text-xs font-medium text-muted-foreground">{title}</h2>
      {children}
    </section>
  )
}
