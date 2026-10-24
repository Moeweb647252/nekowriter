/** The changes page: a filterable log (chapter, session, target type, actor) with a before/after
 * diff per row and Revert (PROJECT §8.8, spec/06 §3). */
import { useState } from "react"
import { useParams } from "react-router"
import { useTranslation } from "react-i18next"

import type { ChangeActor } from "@/api/gen/ChangeActor"
import type { ChangeTargetType } from "@/api/gen/ChangeTargetType"
import { useChanges } from "@/api/hooks/useCanon"
import { useChapters, useTracks } from "@/api/hooks/tracks"
import { ChangeList } from "@/features/canon/ChangeList"
import { EmptyState } from "@/components/common/EmptyState"
import { Skeleton } from "@/components/ui/skeleton"

const TARGET_TYPES: ChangeTargetType[] = [
  "entity",
  "relation",
  "appearance_set",
  "timeline_event",
  "memory_note",
  "foreshadowing",
  "chapter_summary",
  "setting_doc",
  "character_design",
]
const ACTORS: ChangeActor[] = ["user", "agent", "system"]

export default function ChangesPage() {
  const { t } = useTranslation()
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const [chapterId, setChapterId] = useState<number | undefined>(undefined)
  const [targetType, setTargetType] = useState<ChangeTargetType | undefined>(undefined)
  const [actor, setActor] = useState<ChangeActor | undefined>(undefined)
  const changes = useChanges(bookId, { chapterId, targetType, actor })

  const tracks = useTracks(bookId)
  const main = tracks.data?.find((track) => track.kind === "main")
  const chapters = useChapters(main?.id ?? 0)

  if (changes.isPending) {
    return <Skeleton className="m-6 h-64" />
  }

  return (
    <div className="mx-auto max-w-4xl p-6">
      <header className="mb-3">
        <h1 className="text-xl font-semibold">{t("book:changes")}</h1>
        <p className="text-xs text-muted-foreground">{t("canon:changes_hint")}</p>
      </header>
      <div className="mb-3 flex flex-wrap items-center gap-1">
        <select
          value={chapterId ?? ""}
          onChange={(event) =>
            setChapterId(event.target.value === "" ? undefined : Number(event.target.value))
          }
          className="h-8 rounded-md border bg-background px-2 text-xs"
          aria-label={t("canon:filter_chapter")}
        >
          <option value="">{t("canon:all_chapters")}</option>
          {(chapters.data ?? []).map((chapter) => (
            <option key={chapter.id} value={chapter.id}>
              {t("canon:chapter_number_title", {
                number: chapter.number,
                title: chapter.title,
              })}
            </option>
          ))}
        </select>
        <select
          value={targetType ?? ""}
          onChange={(event) =>
            setTargetType(
              (event.target.value || undefined) as ChangeTargetType | undefined,
            )
          }
          className="h-8 rounded-md border bg-background px-2 text-xs"
          aria-label={t("canon:filter_target_type")}
        >
          <option value="">{t("canon:all_target_types")}</option>
          {TARGET_TYPES.map((value) => (
            <option key={value} value={value}>
              {t(`book:change_target.${value}`)}
            </option>
          ))}
        </select>
        <select
          value={actor ?? ""}
          onChange={(event) =>
            setActor((event.target.value || undefined) as ChangeActor | undefined)
          }
          className="h-8 rounded-md border bg-background px-2 text-xs"
          aria-label={t("canon:filter_actor")}
        >
          <option value="">{t("canon:all_actors")}</option>
          {ACTORS.map((value) => (
            <option key={value} value={value}>
              {t(`canon:change_actor.${value}`)}
            </option>
          ))}
        </select>
      </div>
      {changes.data === undefined || changes.data.items.length === 0 ? (
        <EmptyState title={t("book:no_changes")} />
      ) : (
        <ChangeList bookId={bookId} changes={changes.data.items} />
      )}
    </div>
  )
}
