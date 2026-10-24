/** The entity detail pane (PROJECT §8.8): header, aliases, summary, description, state,
 * attributes, relations, appearances and history. */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import ReactMarkdown from "react-markdown"
import remarkGfm from "remark-gfm"
import { Plus, Trash2 } from "lucide-react"

import type { EntityDto } from "@/api/gen/EntityDto"
import type { EntityPatch } from "@/api/gen/EntityPatch"
import type { RelationDto } from "@/api/gen/RelationDto"
import {
  useDeleteEntity,
  useEntities,
  useEntity,
  usePatchEntity,
  useCreateRelation,
  useDeleteRelation,
  useSetProtagonist,
} from "@/api/hooks/useCanon"
import { useErrorToast } from "@/components/common/ErrorToast"
import { ChangeList } from "@/features/canon/ChangeList"
import { Importance } from "@/features/canon/EntitiesPage"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
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
  const patch = usePatchEntity(bookId)
  const remove = useDeleteEntity(bookId)
  const protagonist = useSetProtagonist(bookId)

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

      <RelationsEditor bookId={bookId} entity={data} />
      <Appearances entity={data} />

      <section>
        <h3 className="mb-1 text-xs font-medium text-muted-foreground">
          {t("canon:history")}
        </h3>
        <ChangeList bookId={bookId} changes={data.recent_changes} compact />
      </section>
    </div>
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

/** The key/value attribute grid (PROJECT §8.8). */
function AttributesEditor({
  attributes,
  onChange,
}: {
  attributes: Record<string, string>
  onChange: (next: Record<string, string>) => void
}) {
  const { t } = useTranslation()
  const [key, setKey] = useState("")
  const [value, setValue] = useState("")
  const entries = Object.entries(attributes)
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
              value={entryValue}
              onChange={(event) => onChange({ ...attributes, [entryKey]: event.target.value })}
              className="h-7 text-sm"
            />
            <Button
              size="sm"
              variant="ghost"
              aria-label={t("canon:remove_attribute")}
              onClick={() => {
                const next = { ...attributes }
                delete next[entryKey]
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
function RelationsEditor({ bookId, entity }: { bookId: number; entity: EntityDto }) {
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
              entity={entity}
              relation={relation}
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
  entity,
  relation,
  nameOf,
  onDelete,
}: {
  entity: EntityDto
  relation: RelationDto
  nameOf: (id: number) => string
  onDelete: () => void
}) {
  const { t } = useTranslation()
  const isSource = relation.source_id === entity.id
  const kind = isSource || relation.inverse_kind === "" ? relation.kind : relation.inverse_kind
  const other = isSource ? relation.target_id : relation.source_id
  return (
    <div className="flex items-center gap-2 text-sm">
      <span className="text-muted-foreground">{kind}</span>
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
        aria-label={t("canon:remove_relation")}
        onClick={onDelete}
      >
        <Trash2 className="size-3" aria-hidden="true" />
      </Button>
    </div>
  )
}

/** The appearances as chapter chips (PROJECT §8.8). */
function Appearances({ entity }: { entity: EntityDto }) {
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
            {t("canon:chapter_number", { number: appearance.chapter_id })} ·{" "}
            {t(`canon:appearance_role.${appearance.role}`)}
          </span>
        ))}
      </div>
    </section>
  )
}
