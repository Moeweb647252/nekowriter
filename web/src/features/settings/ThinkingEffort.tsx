/** The Thinking effort section of the model dialog (spec/04 §5.1, D-162).
 *
 * A profile owns an ordered list of levels (at most 8) plus a default. The combobox offers the
 * protocol's preset suggestions and accepts free text; the list is reorderable with dnd-kit. A
 * protocol switch keeps every stored level and flags the ones the new protocol cannot send instead
 * of deleting them silently.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core"
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import { GripVertical, Plus, TriangleAlert, X } from "lucide-react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { Protocol } from "@/api/gen/Protocol"
import {
  MAX_EFFORT_LEVELS,
  addEffortLevel,
  effortSuggestionsFor,
  reorderEffortLevels,
  unsendableEffortLevels,
  type EffortAddError,
} from "@/features/settings/effortLevels"

/** The Select value for "no default"; invalid as a level token, so it cannot collide. */
const DEFAULT_EFFORT_NONE = "$none"

export function ThinkingEffortSection({
  protocol,
  levels,
  defaultEffort,
  ignoredEffortLevels,
  onChange,
}: {
  protocol: Protocol | null
  levels: string[]
  defaultEffort: string | null
  ignoredEffortLevels: readonly string[]
  onChange: (levels: string[], defaultEffort: string | null) => void
}) {
  const { t } = useTranslation()
  const [error, setError] = useState<EffortAddError | null>(null)
  const fromServer = ignoredEffortLevels.filter((level) => levels.includes(level))
  const unsendable = [
    ...new Set([...unsendableEffortLevels(levels, protocol), ...fromServer]),
  ]
  const protocolName = protocol === null ? "" : t(`settings:protocol.${protocol}`, { defaultValue: protocol })

  const add = (raw: string) => {
    const result = addEffortLevel(levels, raw)
    setError(result.error)
    if (result.error === null) {
      onChange(result.levels, defaultEffort)
    }
  }
  const remove = (level: string) => {
    setError(null)
    onChange(
      levels.filter((candidate) => candidate !== level),
      defaultEffort === level ? null : defaultEffort,
    )
  }
  const removeUnsupported = () => {
    const remaining = levels.filter((level) => !unsendable.includes(level))
    onChange(remaining, unsendable.includes(defaultEffort ?? "") ? null : defaultEffort)
  }
  const reorder = (event: DragEndEvent) => {
    const { active, over } = event
    if (over === null || active.id === over.id) {
      return
    }
    const oldIndex = levels.indexOf(String(active.id))
    const newIndex = levels.indexOf(String(over.id))
    if (oldIndex < 0 || newIndex < 0) {
      return
    }
    onChange(reorderEffortLevels(levels, oldIndex, newIndex), defaultEffort)
  }
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )

  return (
    <div className="space-y-2" data-testid="thinking-effort">
      <div className="space-y-1">
        <Label>{t("settings:effort.title")}</Label>
        <p className="text-xs text-muted-foreground">{t("settings:effort.hint")}</p>
      </div>

      {protocol === "mock" ? (
        <p className="text-xs text-muted-foreground" data-testid="effort-mock-note">
          {t("settings:effort.mock_note")}
        </p>
      ) : (
        <EffortCombobox
          suggestions={effortSuggestionsFor(protocol, levels)}
          disabled={protocol === null}
          onAdd={add}
        />
      )}

      {levels.length === 0 ? null : (
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={reorder}>
          <SortableContext items={levels} strategy={verticalListSortingStrategy}>
            <ul className="space-y-1" aria-label={t("settings:effort.levels")}>
              {levels.map((level) => (
                <SortableEffortLevel
                  key={level}
                  level={level}
                  unsendable={unsendable.includes(level)}
                  onRemove={() => remove(level)}
                />
              ))}
            </ul>
          </SortableContext>
        </DndContext>
      )}

      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>{t("settings:effort.count", { current: levels.length, max: MAX_EFFORT_LEVELS })}</span>
        {error === null ? null : (
          <span role="alert" data-testid="effort-error" className="text-status-failed">
            {t(`settings:effort.error.${error}`, { max: MAX_EFFORT_LEVELS })}
          </span>
        )}
      </div>

      {unsendable.length === 0 ? null : (
        <Alert variant="destructive" data-testid="ignored-effort-alert">
          <TriangleAlert />
          <AlertTitle>{t("settings:effort.ignored_title")}</AlertTitle>
          <AlertDescription>
            <p>
              {t("settings:effort.ignored_body", {
                protocol: protocolName,
                levels: unsendable.join(", "),
              })}
            </p>
            <Button variant="outline" size="sm" className="mt-2" onClick={removeUnsupported}>
              {t("settings:effort.ignored_fix")}
            </Button>
          </AlertDescription>
        </Alert>
      )}

      {levels.length === 0 || protocol === "mock" ? null : (
        <div className="space-y-1.5">
          <Label htmlFor="model-effort-default">{t("settings:effort.default")}</Label>
          <Select
            value={defaultEffort ?? DEFAULT_EFFORT_NONE}
            onValueChange={(value) =>
              onChange(levels, value === DEFAULT_EFFORT_NONE ? null : value)
            }
          >
            <SelectTrigger id="model-effort-default" aria-label={t("settings:effort.default")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={DEFAULT_EFFORT_NONE}>{t("settings:effort.default_none")}</SelectItem>
              {levels.map((level) => (
                <SelectItem key={level} value={level}>
                  {t(`settings:value.effort.${level}`, { defaultValue: level })}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}
    </div>
  )
}

/** The type-to-add control: a text input with the protocol's suggestions below it. */
function EffortCombobox({
  suggestions,
  disabled,
  onAdd,
}: {
  suggestions: readonly string[]
  disabled: boolean
  onAdd: (value: string) => void
}) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")
  const [open, setOpen] = useState(false)
  const visible = suggestions.filter((level) =>
    level.toLowerCase().includes(query.trim().toLowerCase()),
  )

  const submit = () => {
    onAdd(query)
    setQuery("")
    setOpen(false)
  }

  return (
    <div className="space-y-1">
      <div className="flex items-center gap-2">
        <div className="relative flex-1">
          <Input
            role="combobox"
            aria-label={t("settings:effort.combobox")}
            aria-autocomplete="list"
            aria-expanded={open && visible.length > 0}
            aria-controls="effort-suggestions"
            placeholder={t("settings:effort.placeholder")}
            value={query}
            disabled={disabled}
            onFocus={() => setOpen(true)}
            onChange={(event) => {
              setQuery(event.target.value)
              setOpen(true)
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault()
                submit()
              }
              if (event.key === "Escape") {
                setOpen(false)
              }
            }}
          />
          {open && visible.length > 0 ? (
            <div
              id="effort-suggestions"
              role="listbox"
              className="absolute z-50 mt-1 max-h-48 w-full overflow-y-auto rounded-md border bg-popover p-1 shadow-md"
            >
              {visible.map((level) => (
                <button
                  key={level}
                  type="button"
                  role="option"
                  aria-selected={false}
                  className="block w-full rounded-sm px-2 py-1 text-left text-sm hover:bg-accent"
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => {
                    onAdd(level)
                    setQuery("")
                    setOpen(false)
                  }}
                >
                  {t(`settings:value.effort.${level}`, { defaultValue: level })}
                </button>
              ))}
            </div>
          ) : null}
        </div>
        <Button type="button" variant="outline" size="sm" disabled={disabled} onClick={submit}>
          <Plus className="size-3.5" />
          {t("settings:effort.add")}
        </Button>
      </div>
    </div>
  )
}

/** One sortable level row. */
function SortableEffortLevel({
  level,
  unsendable,
  onRemove,
}: {
  level: string
  unsendable: boolean
  onRemove: () => void
}) {
  const { t } = useTranslation()
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: level,
  })
  return (
    <li
      ref={setNodeRef}
      data-testid={`effort-level-${level}`}
      data-unsupported={unsendable ? "true" : "false"}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={isDragging ? "opacity-60" : undefined}
    >
      <div className="flex items-center gap-2 rounded-md border px-2 py-1">
        <button
          type="button"
          className="cursor-grab text-muted-foreground/50 hover:text-muted-foreground"
          aria-label={t("settings:effort.drag", { level })}
          {...attributes}
          {...listeners}
        >
          <GripVertical className="size-3.5" />
        </button>
        <span className="text-sm">{level}</span>
        {unsendable ? <Badge variant="destructive">{t("settings:effort.unsupported")}</Badge> : null}
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="ml-auto"
          aria-label={t("settings:effort.remove", { level })}
          onClick={onRemove}
        >
          <X className="size-3.5" />
        </Button>
      </div>
    </li>
  )
}
