import { useState } from "react"
import { useTranslation } from "react-i18next"

import { EntriesPanel } from "./EntriesPanel"
import { EntryEditor } from "./EntryEditor"
import { SearchPanel } from "./SearchPanel"
import { SpacesPanel } from "./SpacesPanel"
import type { KnowledgeEntryId } from "@/api/generated/KnowledgeEntryId"
import type { KnowledgeSpaceId } from "@/api/generated/KnowledgeSpaceId"
import { useSpaces } from "@/api/queries/knowledge"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Skeleton } from "@/components/ui/skeleton"

/** What the editor shows: nothing, a new entry, or an existing one. */
export type EditorTarget =
  { kind: "none" } | { kind: "new" } | { kind: "entry"; id: KnowledgeEntryId }

/** Spaces, their entries and an entry's editor (§12.2). */
export function KnowledgePage() {
  const { t } = useTranslation()
  const spaces = useSpaces()
  const [space, setSpace] = useState<KnowledgeSpaceId | null>(null)
  const [target, setTarget] = useState<EditorTarget>({ kind: "none" })

  return (
    <div className="flex flex-col gap-6">
      <div className="flex max-w-2xl flex-col gap-1">
        <h1 className="font-heading text-2xl font-semibold">{t(($) => $.knowledge.title)}</h1>
        <p className="text-sm text-muted-foreground">{t(($) => $.knowledge.description)}</p>
      </div>
      {spaces.isPending && <Skeleton className="h-96" />}
      {spaces.isError && <ErrorAlert error={spaces.error} onRetry={() => void spaces.refetch()} />}
      <SearchPanel space={space} onOpen={(id) => setTarget({ kind: "entry", id })} />
      {spaces.isSuccess && (
        <div className="grid gap-6 lg:grid-cols-[13rem_16rem_1fr]">
          <SpacesPanel
            spaces={spaces.data}
            selected={space}
            onSelect={(next) => {
              setSpace(next)
              setTarget({ kind: "none" })
            }}
          />
          <EntriesPanel
            space={space}
            canCreate={spaces.data.length > 0}
            target={target}
            onTarget={setTarget}
          />
          <EntryEditor
            // Each entry, and the new one, gets a fresh form.
            key={target.kind === "entry" ? target.id : target.kind}
            spaces={spaces.data}
            defaultSpace={space}
            target={target}
            onSaved={(id) => setTarget({ kind: "entry", id })}
            onDeleted={() => setTarget({ kind: "none" })}
          />
        </div>
      )}
    </div>
  )
}
