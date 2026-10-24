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
  const chapters = useChapters(main?.id ?? 0)

  // "Next chapter of: Main story" or a specific chapter (PROJECT §8.8).
  const [chapterId, setChapterId] = useState<number | undefined>(undefined)
  const memory = useMemory(bookId, { chapterId })

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

  return (
    <div className="mx-auto max-w-6xl space-y-4 p-6">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold">{t("canon:memory")}</h1>
          <p className="text-xs text-muted-foreground">{t("canon:memory_hint")}</p>
        </div>
        <label className="flex items-center gap-2 text-xs text-muted-foreground">
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
      </header>

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

        <MemoryColumn title={t("canon:memory_long_term")}>
          {snapshot.key_notes.length === 0 ? null : (
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
          )}
          {snapshot.open_foreshadowing.length === 0 ? null : (
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
          <p className="mt-2 text-[0.65rem] text-muted-foreground">{t("canon:memory_cast")}</p>
          <ul className="space-y-0.5 text-xs">
            {snapshot.cast.map((item) => (
              <li key={item.id}>
                #{item.id} {item.name}【{t(`canon:kind.${item.kind}`)}】
                {item.summary === "" ? "" : ` — ${item.summary}`}
              </li>
            ))}
          </ul>
          <p className="mt-2 text-[0.65rem] text-muted-foreground">
            {t("canon:memory_counts", {
              character: snapshot.counts.character,
              organization: snapshot.counts.organization,
              location: snapshot.counts.location,
              item: snapshot.counts.item,
              other: snapshot.counts.other,
              event: snapshot.counts.event,
              fact: snapshot.counts.fact,
              world_rule: snapshot.counts.world_rule,
              foreshadowing_open: snapshot.counts.foreshadowing_open,
            })}
          </p>
        </MemoryColumn>
      </div>

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
