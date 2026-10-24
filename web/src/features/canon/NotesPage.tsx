/** The memory notes page: the facts and world rules table with kind and status filters
 * (PROJECT §8.8, spec/10 §2.4). */
import { useState } from "react"
import { useParams } from "react-router"
import { useTranslation } from "react-i18next"
import { Plus, Trash2 } from "lucide-react"

import type { MemoryNoteDto } from "@/api/gen/MemoryNoteDto"
import type { NoteKind } from "@/api/gen/NoteKind"
import {
  useCreateMemoryNote,
  useDeleteMemoryNote,
  useMemoryNotes,
  usePatchMemoryNote,
} from "@/api/hooks/useCanon"
import { useErrorToast } from "@/components/common/ErrorToast"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"

const KINDS: NoteKind[] = ["fact", "world_rule", "other"]

export default function NotesPage() {
  const { t } = useTranslation()
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const [kind, setKind] = useState<NoteKind | undefined>(undefined)
  const [status, setStatus] = useState<"active" | "retired" | undefined>("active")
  const notes = useMemoryNotes(bookId, { kind, status })
  const create = useCreateMemoryNote(bookId)
  const patch = usePatchMemoryNote(bookId)
  const remove = useDeleteMemoryNote(bookId)
  const showError = useErrorToast()
  const [draft, setDraft] = useState("")
  const [draftKind, setDraftKind] = useState<NoteKind>("fact")

  if (notes.isPending) {
    return <Skeleton className="m-6 h-64" />
  }
  const rows = notes.data ?? []

  return (
    <div className="mx-auto max-w-4xl p-6">
      <header className="mb-3 flex flex-wrap items-end justify-between gap-2">
        <div>
          <h1 className="text-xl font-semibold">{t("canon:notes")}</h1>
          <p className="text-xs text-muted-foreground">{t("canon:notes_hint")}</p>
        </div>
        <div className="flex flex-wrap gap-1">
          <Chip
            active={kind === undefined}
            onClick={() => setKind(undefined)}
            label={t("canon:all_kinds")}
          />
          {KINDS.map((value) => (
            <Chip
              key={value}
              active={kind === value}
              onClick={() => setKind(kind === value ? undefined : value)}
              label={t(`canon:note_kind.${value}`)}
            />
          ))}
          <Chip
            active={status === "retired"}
            onClick={() => setStatus(status === "retired" ? "active" : "retired")}
            label={t("canon:note_status.retired")}
          />
        </div>
      </header>

      <form
        className="mb-3 flex flex-wrap items-center gap-1"
        onSubmit={(event) => {
          event.preventDefault()
          if (draft.trim() === "") {
            return
          }
          create.mutate(
            { kind: draftKind, content: draft.trim(), importance: 3, entity_ids: [] },
            {
              onSuccess: () => setDraft(""),
              onError: showError,
            },
          )
        }}
      >
        <select
          value={draftKind}
          onChange={(event) => setDraftKind(event.target.value as NoteKind)}
          className="h-8 rounded-md border bg-background px-2 text-xs"
          aria-label={t("canon:note_kind_label")}
        >
          {KINDS.map((value) => (
            <option key={value} value={value}>
              {t(`canon:note_kind.${value}`)}
            </option>
          ))}
        </select>
        <Input
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          placeholder={t("canon:note_placeholder")}
          className="h-8 flex-1"
        />
        <Button size="sm" type="submit" disabled={draft.trim() === ""}>
          <Plus className="size-3.5" aria-hidden="true" />
          {t("canon:add_note")}
        </Button>
      </form>

      {rows.length === 0 ? (
        <EmptyState title={t("canon:no_notes")} description={t("canon:no_notes_hint")} />
      ) : (
        <table className="w-full text-sm">
          <thead className="text-left text-xs text-muted-foreground">
            <tr className="border-b">
              <th className="px-2 py-1.5 font-medium">{t("canon:column_kind")}</th>
              <th className="px-2 py-1.5 font-medium">{t("canon:column_content")}</th>
              <th className="px-2 py-1.5 font-medium">{t("canon:column_status")}</th>
              <th className="px-2 py-1.5" />
            </tr>
          </thead>
          <tbody>
            {rows.map((note) => (
              <NoteRow
                key={note.id}
                note={note}
                onRetire={() =>
                  patch.mutate(
                    { id: note.id, patch: { status: "retired", note: t("canon:retire_reason") } },
                    { onError: showError },
                  )
                }
                onRestore={() =>
                  patch.mutate(
                    { id: note.id, patch: { status: "active" } },
                    { onError: showError },
                  )
                }
                onDelete={() => remove.mutate(note.id, { onError: showError })}
              />
            ))}
          </tbody>
        </table>
      )}
    </div>
  )
}

/** One note row. */
function NoteRow({
  note,
  onRetire,
  onRestore,
  onDelete,
}: {
  note: MemoryNoteDto
  onRetire: () => void
  onRestore: () => void
  onDelete: () => void
}) {
  const { t } = useTranslation()
  return (
    <tr className={cn("border-b last:border-0", note.status === "retired" && "opacity-60")}>
      <td className="px-2 py-1.5 text-xs text-muted-foreground">
        {t(`canon:note_kind.${note.kind}`)}
      </td>
      <td className="px-2 py-1.5">{note.content}</td>
      <td className="px-2 py-1.5 text-xs text-muted-foreground">
        {t(`canon:note_status.${note.status}`)}
      </td>
      <td className="px-2 py-1.5 text-right">
        <Button size="sm" variant="ghost" onClick={note.status === "active" ? onRetire : onRestore}>
          {note.status === "active" ? t("canon:retire") : t("canon:restore")}
        </Button>
        <Button size="sm" variant="ghost" aria-label={t("common:delete")} onClick={onDelete}>
          <Trash2 className="size-3" aria-hidden="true" />
        </Button>
      </td>
    </tr>
  )
}

/** One filter chip. */
function Chip({
  active,
  onClick,
  label,
}: {
  active: boolean
  onClick: () => void
  label: string
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded-full border px-2 py-0.5 text-xs",
        active
          ? "border-primary bg-primary/10 text-primary"
          : "text-muted-foreground hover:bg-accent",
      )}
    >
      {label}
    </button>
  )
}
