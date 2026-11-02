/** The entity detail pane (PROJECT §8.8): header, aliases, summary, description, state,
 * attributes, relations, appearances and history. */
import { useState } from "react"
import { Link } from "react-router"
import { useTranslation } from "react-i18next"
import ReactMarkdown from "react-markdown"
import remarkGfm from "remark-gfm"
import { MoreHorizontal, Plus, Trash2 } from "lucide-react"

import type { EntityDto } from "@/api/gen/EntityDto"
import type { EntityPatch } from "@/api/gen/EntityPatch"
import type { RelationDto } from "@/api/gen/RelationDto"
import type { RelationPatch } from "@/api/gen/RelationPatch"
import type { RelationStatus } from "@/api/gen/RelationStatus"
import {
  useDeleteEntity,
  useEntities,
  useEntity,
  useEntityHistory,
  usePatchEntity,
  usePatchRelation,
  useCreateRelation,
  useDeleteRelation,
  useSetProtagonist,
} from "@/api/hooks/useCanon"
import { useTracks } from "@/api/hooks/tracks"
import { useErrorToast } from "@/components/common/ErrorToast"
import { ChangeList } from "@/features/canon/ChangeList"
import { EntityMergeDialog } from "@/features/canon/EntityMergeDialog"
import { GoalsReferencedBy } from "@/features/canon/GoalsReferencedBy"
import {
  chapterRefLabel,
  useChapterIndex,
  type ChapterIndex,
} from "@/features/canon/chapterIndex"
import { Importance } from "@/features/canon/EntitiesPage"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Skeleton } from "@/components/ui/skeleton"
import { isComposing } from "@/lib/hotkeys"
import { Textarea } from "@/components/ui/textarea"

export type EntityDetailProps = {
  bookId: number
  entityId: number
}

/** The entity detail pane. */
export function EntityDetail({ bookId, entityId }: EntityDetailProps) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const entity = useEntity(entityId)
  const tracks = useTracks(bookId)
  const chapterIndex = useChapterIndex(tracks.data)
  const patch = usePatchEntity(bookId)
  const remove = useDeleteEntity(bookId)
  const protagonist = useSetProtagonist(bookId)
  const [mergeOpen, setMergeOpen] = useState(false)

  if (entity.isPending) {
    return <Skeleton className="m-4 h-64" />
  }
  if (entity.isError || entity.data === undefined) {
    return <p className="p-4 text-sm text-muted-foreground">{t("common:error_body")}</p>
  }
  const data = entity.data

  const save = (change: EntityPatch) =>
    patch.mutate(
      { id: entityId, patch: change },
      { onError: (error) => showError(error) },
    )

  return (
    <div className="space-y-4 p-4">
      <header className="space-y-1">
        <div className="flex items-start justify-between gap-2">
          <div>
            <h2 className="text-base font-semibold">{data.name}</h2>
            <p className="text-xs text-muted-foreground">
              {t(`canon:kind.${data.kind}`)} · {t(`canon:status.${data.status}`)} ·{" "}
              <Importance value={data.importance} />
            </p>
          </div>
          <div className="flex gap-1">
            <Button
              size="sm"
              variant="outline"
              onClick={() =>
                protagonist.mutate(entityId, { onError: (error) => showError(error) })
              }
            >
              {t("canon:make_protagonist")}
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button size="sm" variant="ghost" aria-label={t("canon:entity_actions")}>
                  <MoreHorizontal className="size-3.5" aria-hidden="true" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={() => setMergeOpen(true)}>
                  {t("canon:entity_merge.action")}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => remove.mutate(entityId, { onError: (error) => showError(error) })}
            >
              <Trash2 className="size-3.5" aria-hidden="true" />
            </Button>
          </div>
        </div>
        {data.aliases.length === 0 ? null : (
          <div className="flex flex-wrap gap-1">
            {data.aliases.map((alias) => (
              <span
                key={alias}
                className="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground"
              >
                {alias}
              </span>
            ))}
          </div>
        )}
        {data.summary === "" ? null : (
          <p className="text-sm text-muted-foreground">{data.summary}</p>
        )}
      </header>

      <EditableField
        label={t("canon:field_summary")}
        value={data.summary}
        onSave={(summary) => save({ summary })}
      />

      <section>
        <h3 className="mb-1 text-xs font-medium text-muted-foreground">
          {t("canon:field_state")}
        </h3>
        <div className="rounded-md border border-primary/30 bg-primary/5 p-2">
          <EditableField
            label=""
            value={data.state}
            onSave={(state) => save({ state })}
            multiline
          />
        </div>
      </section>

      <section>
        <h3 className="mb-1 text-xs font-medium text-muted-foreground">
          {t("canon:field_description")}
        </h3>
        {data.description === "" ? (
          <p className="text-xs text-muted-foreground">{t("canon:no_description")}</p>
        ) : (
          <div className="prose-latin text-sm">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{data.description}</ReactMarkdown>
          </div>
        )}
      </section>

      <AttributesEditor
        attributes={data.attributes}
        onChange={(attributes) => save({ attributes })}
      />

      <RelationsEditor
        bookId={bookId}
        entity={data}
        chapterIndex={chapterIndex}
      />
      <Appearances entity={data} chapterIndex={chapterIndex} />

      <GoalsReferencedBy bookId={bookId} entityId={entityId} />

      <HistorySection
        bookId={bookId}
        entityId={entityId}
        recentChanges={data.recent_changes}
      />

      {mergeOpen ? (
        <EntityMergeDialog
          bookId={bookId}
          target={data}
          open
          onOpenChange={setMergeOpen}
        />
      ) : null}
    </div>
  )
}

/** The detail pane's history preview length; more rows mean the preview is truncated (spec/08 §8.8). */
export const DETAIL_CHANGE_LIMIT = 20

/** The entity history preview and the full-history toggle (spec/06 §3, spec/11 §5.5, D-161). */
export function HistorySection({
  bookId,
  entityId,
  recentChanges,
}: {
  bookId: number
  entityId: number
  recentChanges: EntityDto["recent_changes"]
}) {
  const { t } = useTranslation()
  // The expansion resets on every entity switch, including A → B → A: remembering A would
  // silently re-open its history on the way back (plan §2.5).
  const [expandedFor, setExpandedFor] = useState<number | null>(null)
  const [previousEntityId, setPreviousEntityId] = useState(entityId)
  if (previousEntityId !== entityId) {
    setPreviousEntityId(entityId)
    setExpandedFor(null)
  }
  const expanded = expandedFor === entityId
  const history = useEntityHistory(expanded ? entityId : undefined)
  return (
    <section>
      <div className="mb-1 flex items-center justify-between gap-2">
        <h3 className="text-xs font-medium text-muted-foreground">
          {t("canon:history")}
        </h3>
        {!expanded && recentChanges.length === DETAIL_CHANGE_LIMIT ? (
          <Button
            size="sm"
            variant="outline"
            onClick={() => setExpandedFor(entityId)}
          >
            {t("canon:show_full_history")}
          </Button>
        ) : null}
      </div>
      {expanded ? (
        history.isPending ? (
          <Skeleton className="h-24" />
        ) : (
          <>
            <ChangeList bookId={bookId} changes={history.data ?? []} compact />
            {history.data !== undefined && history.data.length >= 100 ? (
              <p className="mt-2 text-xs text-muted-foreground">
                {t("canon:history_capped", { count: 100 })}{" "}
                <Link
                  to={`/books/${bookId}/canon/changes`}
                  className="text-primary underline-offset-2 hover:underline"
                >
                  {t("canon:history_capped_link")}
                </Link>
              </p>
            ) : null}
          </>
        )
      ) : (
        <ChangeList bookId={bookId} changes={recentChanges} compact />
      )}
    </section>
  )
}

/** One inline-editable text field (click to edit, blur or ⌘⏎ to save). */
function EditableField({
  label,
  value,
  onSave,
  multiline = false,
}: {
  label: string
  value: string
  onSave: (next: string) => void
  multiline?: boolean
}) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState<string | null>(null)
  const shown = draft ?? value
  const commit = () => {
    if (draft !== null && draft !== value) {
      onSave(draft)
    }
    setDraft(null)
  }
  return (
    <div className="space-y-1">
      {label === "" ? null : (
        <label className="text-xs font-medium text-muted-foreground">{label}</label>
      )}
      {multiline ? (
        <Textarea
          value={shown}
          rows={2}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (isComposing(event)) {
              return
            }
            if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
              commit()
            }
          }}
          placeholder={t("canon:field_placeholder")}
          className="min-h-0 border-0 bg-transparent p-0 text-sm shadow-none focus-visible:ring-0"
        />
      ) : (
        <Input
          value={shown}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (isComposing(event)) {
              return
            }
            if (event.key === "Enter") {
              commit()
            }
          }}
          placeholder={t("canon:field_placeholder")}
          className="h-7 border-0 bg-transparent px-0 text-sm shadow-none focus-visible:ring-0"
        />
      )}
    </div>
  )
}

/** The key/value attribute grid (PROJECT §8.8).
 *
 * Edits are kept locally and saved on blur or Enter: every save is a change-log row (spec/06
 * §3), so a patch per keystroke would fill the entity's history with half-typed values.
 */
export function AttributesEditor({
  attributes,
  onChange,
}: {
  attributes: Record<string, string>
  onChange: (next: Record<string, string>) => void
}) {
  const { t } = useTranslation()
  const [key, setKey] = useState("")
  const [value, setValue] = useState("")
  const [drafts, setDrafts] = useState<Record<string, string>>({})
  const entries = Object.entries(attributes)
  const commit = (name: string) => {
    const draft = drafts[name]
    if (draft === undefined) {
      return
    }
    setDrafts((current) => {
      const next = { ...current }
      delete next[name]
      return next
    })
    if (draft !== attributes[name]) {
      onChange({ ...attributes, [name]: draft })
    }
  }
  return (
    <section>
      <h3 className="mb-1 text-xs font-medium text-muted-foreground">
        {t("canon:field_attributes")}
      </h3>
      <div className="space-y-1">
        {entries.map(([entryKey, entryValue]) => (
          <div key={entryKey} className="flex items-center gap-2 text-sm">
            <span className="w-24 shrink-0 truncate text-xs text-muted-foreground">
              {entryKey}
            </span>
            <Input
              value={drafts[entryKey] ?? entryValue}
              onChange={(event) =>
                setDrafts((current) => ({ ...current, [entryKey]: event.target.value }))
              }
              onBlur={() => commit(entryKey)}
              onKeyDown={(event) => {
                if (isComposing(event)) {
                  return
                }
                if (event.key === "Enter") {
                  commit(entryKey)
                }
              }}
              className="h-7 text-sm"
            />
            <Button
              size="sm"
              variant="ghost"
              aria-label={t("canon:remove_attribute")}
              onClick={() => {
                const next = { ...attributes }
                delete next[entryKey]
                setDrafts((current) => {
                  const rest = { ...current }
                  delete rest[entryKey]
                  return rest
                })
                onChange(next)
              }}
            >
              <Trash2 className="size-3" aria-hidden="true" />
            </Button>
          </div>
        ))}
        <div className="flex items-center gap-2">
          <Input
            value={key}
            onChange={(event) => setKey(event.target.value)}
            placeholder={t("canon:attribute_key")}
            className="h-7 w-24 text-xs"
          />
          <Input
            value={value}
            onChange={(event) => setValue(event.target.value)}
            placeholder={t("canon:attribute_value")}
            className="h-7 text-xs"
          />
          <Button
            size="sm"
            variant="outline"
            disabled={key.trim() === ""}
            onClick={() => {
              onChange({ ...attributes, [key.trim()]: value })
              setKey("")
              setValue("")
            }}
          >
            <Plus className="size-3" aria-hidden="true" />
          </Button>
        </div>
      </div>
    </section>
  )
}

/** The relations list with an add-relation row (PROJECT §8.8). */
export function RelationsEditor({
  bookId,
  entity,
  chapterIndex,
}: {
  bookId: number
  entity: EntityDto
  chapterIndex: ChapterIndex
}) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const create = useCreateRelation(bookId)
  const remove = useDeleteRelation(bookId)
  const entities = useEntities(bookId)
  const [otherId, setOtherId] = useState("")
  const [kind, setKind] = useState("")

  const nameOf = (id: number) =>
    entities.data?.find((candidate) => candidate.id === id)?.name ?? `#${id}`

  return (
    <section>
      <h3 className="mb-1 text-xs font-medium text-muted-foreground">
        {t("canon:relations")}
      </h3>
      <div className="space-y-1">
        {entity.relations.length === 0 ? (
          <p className="text-xs text-muted-foreground">{t("canon:no_relations")}</p>
        ) : (
          entity.relations.map((relation) => (
            <RelationRow
              key={relation.id}
              bookId={bookId}
              entity={entity}
              relation={relation}
              chapterIndex={chapterIndex}
              nameOf={nameOf}
              onDelete={() => remove.mutate(relation.id, { onError: showError })}
            />
          ))
        )}
        <div className="flex flex-wrap items-center gap-1 pt-1">
          <select
            value={otherId}
            onChange={(event) => setOtherId(event.target.value)}
            className="h-7 rounded-md border bg-background px-2 text-xs"
            aria-label={t("canon:relation_other")}
          >
            <option value="">{t("canon:relation_other")}</option>
            {(entities.data ?? [])
              .filter((candidate) => candidate.id !== entity.id)
              .map((candidate) => (
                <option key={candidate.id} value={candidate.id}>
                  {candidate.name}
                </option>
              ))}
          </select>
          <Input
            value={kind}
            onChange={(event) => setKind(event.target.value)}
            placeholder={t("canon:relation_kind")}
            className="h-7 w-24 text-xs"
          />
          <Button
            size="sm"
            variant="outline"
            disabled={otherId === "" || kind.trim() === ""}
            onClick={() =>
              create.mutate(
                {
                  source_id: entity.id,
                  target_id: Number(otherId),
                  kind: kind.trim(),
                  inverse_kind: "",
                  description: "",
                },
                {
                  onSuccess: () => {
                    setOtherId("")
                    setKind("")
                  },
                  onError: showError,
                },
              )
            }
          >
            <Plus className="size-3" aria-hidden="true" />
            {t("canon:add_relation")}
          </Button>
        </div>
      </div>
    </section>
  )
}

/** One relation, phrased from this entity's perspective (spec/06 §1.2). */
function RelationRow({
  bookId,
  entity,
  relation,
  chapterIndex,
  nameOf,
  onDelete,
}: {
  bookId: number
  entity: EntityDto
  relation: RelationDto
  chapterIndex: ChapterIndex
  nameOf: (id: number) => string
  onDelete: () => void
}) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const patch = usePatchRelation(bookId)
  const [editing, setEditing] = useState(false)
  // The form edits the stored kind names as they are; the row's label follows the source →
  // target direction (spec/06 §1.2, spec/11 §5.5).
  const [kind, setKind] = useState(relation.kind)
  const [inverseKind, setInverseKind] = useState(relation.inverse_kind)
  const [description, setDescription] = useState(relation.description)
  const [status, setStatus] = useState<RelationStatus>(relation.status)
  const [since, setSince] = useState(
    relation.since_chapter_id === null ? "" : String(relation.since_chapter_id),
  )
  const [until, setUntil] = useState(
    relation.until_chapter_id === null ? "" : String(relation.until_chapter_id),
  )
  const [note, setNote] = useState("")

  const isSource = relation.source_id === entity.id
  const shownKind =
    isSource || relation.inverse_kind === "" ? relation.kind : relation.inverse_kind
  const other = isSource ? relation.target_id : relation.source_id
  const chapterOptions = Array.from(chapterIndex.values()).sort((left, right) => {
    if (left.trackKind !== right.trackKind) {
      return left.trackKind === "main" ? -1 : 1
    }
    if (left.trackId !== right.trackId) {
      return left.trackId - right.trackId
    }
    return left.number - right.number
  })

  const startEdit = () => {
    setKind(relation.kind)
    setInverseKind(relation.inverse_kind)
    setDescription(relation.description)
    setStatus(relation.status)
    setSince(
      relation.since_chapter_id === null ? "" : String(relation.since_chapter_id),
    )
    setUntil(
      relation.until_chapter_id === null ? "" : String(relation.until_chapter_id),
    )
    setNote("")
    setEditing(true)
  }

  const save = () => {
    const next: RelationPatch = {}
    if (kind !== relation.kind) {
      next.kind = kind
    }
    if (inverseKind !== relation.inverse_kind) {
      next.inverse_kind = inverseKind
    }
    if (description !== relation.description) {
      next.description = description
    }
    if (status !== relation.status) {
      next.status = status
    }
    if (since !== (relation.since_chapter_id === null ? "" : String(relation.since_chapter_id))) {
      next.since_chapter_id = since === "" ? null : Number(since)
    }
    if (until !== (relation.until_chapter_id === null ? "" : String(relation.until_chapter_id))) {
      next.until_chapter_id = until === "" ? null : Number(until)
    }
    if (note.trim() !== "") {
      next.note = note.trim()
    }
    if (Object.keys(next).length === 0) {
      setEditing(false)
      return
    }
    patch.mutate(
      { id: relation.id, patch: next },
      {
        onSuccess: () => setEditing(false),
        onError: showError,
      },
    )
  }

  if (editing) {
    return (
      <div className="space-y-2 rounded-md border p-2 text-sm">
        <p className="text-xs text-muted-foreground">
          {t("canon:relation_edit.perspective", {
            source: nameOf(relation.source_id),
            target: nameOf(relation.target_id),
          })}
        </p>
        <div className="grid grid-cols-2 gap-2">
          <div className="space-y-1">
            <Label htmlFor={`relation-kind-${relation.id}`}>
              {t("canon:relation_edit.kind")}
            </Label>
            <Input
              id={`relation-kind-${relation.id}`}
              value={kind}
              onChange={(event) => setKind(event.target.value)}
              className="h-7 text-xs"
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor={`relation-inverse-${relation.id}`}>
              {t("canon:relation_edit.inverse_kind")}
            </Label>
            <Input
              id={`relation-inverse-${relation.id}`}
              value={inverseKind}
              onChange={(event) => setInverseKind(event.target.value)}
              className="h-7 text-xs"
            />
          </div>
        </div>
        <div className="space-y-1">
          <Label htmlFor={`relation-description-${relation.id}`}>
            {t("canon:relation_edit.description")}
          </Label>
          <Textarea
            id={`relation-description-${relation.id}`}
            rows={2}
            value={description}
            onChange={(event) => setDescription(event.target.value)}
          />
        </div>
        <div className="grid grid-cols-3 gap-2">
          <div className="space-y-1">
            <Label htmlFor={`relation-status-${relation.id}`}>
              {t("canon:relation_edit.status")}
            </Label>
            <select
              id={`relation-status-${relation.id}`}
              value={status}
              onChange={(event) => setStatus(event.target.value as RelationStatus)}
              className="h-7 w-full rounded-md border bg-background px-2 text-xs"
            >
              {(["active", "ended"] as RelationStatus[]).map((value) => (
                <option key={value} value={value}>
                  {t(`canon:relation_status.${value}`)}
                </option>
              ))}
            </select>
          </div>
          <div className="space-y-1">
            <Label htmlFor={`relation-since-${relation.id}`}>
              {t("canon:relation_edit.since")}
            </Label>
            <select
              id={`relation-since-${relation.id}`}
              value={since}
              onChange={(event) => setSince(event.target.value)}
              className="h-7 w-full rounded-md border bg-background px-2 text-xs"
            >
              <option value="">{t("canon:relation_edit.no_chapter")}</option>
              {chapterOptions.map((chapter) => (
                <option key={chapter.id} value={chapter.id}>
                  {chapterRefLabel(t, chapterIndex, chapter.id)}
                </option>
              ))}
            </select>
          </div>
          <div className="space-y-1">
            <Label htmlFor={`relation-until-${relation.id}`}>
              {t("canon:relation_edit.until")}
            </Label>
            <select
              id={`relation-until-${relation.id}`}
              value={until}
              onChange={(event) => setUntil(event.target.value)}
              className="h-7 w-full rounded-md border bg-background px-2 text-xs"
            >
              <option value="">{t("canon:relation_edit.no_chapter")}</option>
              {chapterOptions.map((chapter) => (
                <option key={chapter.id} value={chapter.id}>
                  {chapterRefLabel(t, chapterIndex, chapter.id)}
                </option>
              ))}
            </select>
          </div>
        </div>
        <div className="space-y-1">
          <Label htmlFor={`relation-note-${relation.id}`}>
            {t("canon:relation_edit.change_note")}
          </Label>
          <Textarea
            id={`relation-note-${relation.id}`}
            rows={2}
            value={note}
            onChange={(event) => setNote(event.target.value)}
          />
        </div>
        <div className="flex justify-end gap-1">
          <Button size="sm" variant="outline" onClick={() => setEditing(false)}>
            {t("common:cancel")}
          </Button>
          <Button size="sm" onClick={save}>
            {t("common:save")}
          </Button>
        </div>
      </div>
    )
  }

  return (
    <div className="flex items-center gap-2 text-sm">
      <span className="text-muted-foreground">{shownKind}</span>
      <span className="truncate">{nameOf(other)}</span>
      {relation.status === "ended" ? (
        <span className="text-xs text-muted-foreground">
          ({t("canon:relation_status.ended")})
        </span>
      ) : null}
      <Button
        size="sm"
        variant="ghost"
        className="ml-auto"
        onClick={startEdit}
      >
        {t("canon:relation_edit.edit")}
      </Button>
      <Button
        size="sm"
        variant="ghost"
        aria-label={t("canon:remove_relation")}
        onClick={onDelete}
      >
        <Trash2 className="size-3" aria-hidden="true" />
      </Button>
    </div>
  )
}

/** The appearances as chapter chips (PROJECT §8.8). */
function Appearances({
  entity,
  chapterIndex,
}: {
  entity: EntityDto
  chapterIndex: ChapterIndex
}) {
  const { t } = useTranslation()
  if (entity.appearances.length === 0) {
    return (
      <section>
        <h3 className="mb-1 text-xs font-medium text-muted-foreground">
          {t("canon:appearances")}
        </h3>
        <p className="text-xs text-muted-foreground">{t("canon:no_appearances")}</p>
      </section>
    )
  }
  return (
    <section>
      <h3 className="mb-1 text-xs font-medium text-muted-foreground">
        {t("canon:appearances")}
      </h3>
      <div className="flex flex-wrap gap-1">
        {entity.appearances.map((appearance) => (
          <span
            key={appearance.id}
            className="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground"
          >
            {chapterRefLabel(t, chapterIndex, appearance.chapter_id)} ·{" "}
            {t(`canon:appearance_role.${appearance.role}`)}
          </span>
        ))}
      </div>
    </section>
  )
}
