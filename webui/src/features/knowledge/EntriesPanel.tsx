import { PlusIcon } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"

import type { EditorTarget } from "./KnowledgePage"
import type { KnowledgeSpaceId } from "@/api/generated/KnowledgeSpaceId"
import { useEntries } from "@/api/queries/knowledge"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"

/** The entries of a space, or of all, filtered by title or keyword. */
export function EntriesPanel({
  space,
  canCreate,
  target,
  onTarget,
}: {
  space: KnowledgeSpaceId | null
  canCreate: boolean
  target: EditorTarget
  onTarget: (target: EditorTarget) => void
}) {
  const { t } = useTranslation()
  const [filter, setFilter] = useState("")
  const entries = useEntries(space, filter.trim())

  return (
    <section aria-label={t(($) => $.knowledge.entries)} className="flex flex-col gap-3">
      <Input
        aria-label={t(($) => $.knowledge.filter)}
        placeholder={t(($) => $.knowledge.filter)}
        value={filter}
        onChange={(event) => setFilter(event.target.value)}
      />
      <Button
        variant="outline"
        size="sm"
        disabled={!canCreate}
        onClick={() => onTarget({ kind: "new" })}
      >
        <PlusIcon />
        {t(($) => $.knowledge.newEntry)}
      </Button>
      {!canCreate && (
        <p className="text-sm text-muted-foreground">{t(($) => $.knowledge.noSpaces)}</p>
      )}
      {entries.isPending && <Skeleton className="h-40" />}
      {entries.isError && (
        <ErrorAlert error={entries.error} onRetry={() => void entries.refetch()} />
      )}
      {entries.isSuccess &&
        (entries.data.length === 0 ? (
          canCreate && (
            <p className="text-sm text-muted-foreground">{t(($) => $.knowledge.noEntries)}</p>
          )
        ) : (
          <ul className="flex flex-col">
            {entries.data.map((entry) => {
              const active = target.kind === "entry" && target.id === entry.id
              return (
                <li key={entry.id}>
                  <button
                    type="button"
                    aria-current={active ? "true" : undefined}
                    onClick={() => onTarget({ kind: "entry", id: entry.id })}
                    className={cn(
                      "flex w-full flex-col rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted/60",
                      active && "bg-muted",
                    )}
                  >
                    <span className="truncate">{entry.title}</span>
                    {entry.keywords.length > 0 && (
                      <span className="truncate text-xs text-muted-foreground">
                        {entry.keywords.join(" · ")}
                      </span>
                    )}
                    {entry.index.state !== "ready" && (
                      <span
                        className={cn(
                          "text-xs",
                          entry.index.state === "failed"
                            ? "text-destructive"
                            : "text-muted-foreground",
                        )}
                      >
                        {t(($) => $.knowledge.index[entry.index.state])}
                      </span>
                    )}
                  </button>
                </li>
              )
            })}
          </ul>
        ))}
    </section>
  )
}
