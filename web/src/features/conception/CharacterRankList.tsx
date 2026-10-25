/**
 * The ranked character list with drag and drop (PROJECT §8.7, spec/05 §7 `reorder_characters`,
 * spec/11 §5.4): the same dnd-kit pattern as the settings-materials page, in the studio's
 * compact form. Reordering posts the complete id order; the agent is told through the edit report
 * on its next turn (spec/03 §7).
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
  arrayMove,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import { ChevronDown, ChevronRight, GripVertical, Trash2 } from "lucide-react"

import type { CharacterDto } from "@/api/gen/CharacterDto"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"

export function CharacterRankList({
  characters,
  editable,
  busy,
  onReorder,
  onPatch,
  onDelete,
}: {
  characters: CharacterDto[]
  editable: boolean
  busy: boolean
  onReorder: (ids: number[]) => void
  onPatch: (
    id: number,
    patch: { name?: string; role_label?: string; summary?: string; profile?: string },
  ) => void
  onDelete: (id: number) => void
}) {
  const { t } = useTranslation()
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )

  const onDragEnd = (event: DragEndEvent) => {
    const { active, over } = event
    if (over === null || active.id === over.id) {
      return
    }
    const oldIndex = characters.findIndex((character) => character.id === active.id)
    const newIndex = characters.findIndex((character) => character.id === over.id)
    if (oldIndex < 0 || newIndex < 0) {
      return
    }
    onReorder(arrayMove(characters, oldIndex, newIndex).map((character) => character.id))
  }

  if (characters.length === 0) {
    return (
      <p className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
        {t("book:conception.no_characters")}
      </p>
    )
  }

  return (
    <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
      <SortableContext
        items={characters.map((character) => character.id)}
        strategy={verticalListSortingStrategy}
      >
        <ul className="space-y-1.5">
          {characters.map((character) => (
            <CharacterCard
              key={character.id}
              character={character}
              editable={editable}
              busy={busy}
              onPatch={onPatch}
              onDelete={onDelete}
            />
          ))}
        </ul>
      </SortableContext>
    </DndContext>
  )
}

function CharacterCard({
  character,
  editable,
  busy,
  onPatch,
  onDelete,
}: {
  character: CharacterDto
  editable: boolean
  busy: boolean
  onPatch: (
    id: number,
    patch: { name?: string; role_label?: string; summary?: string; profile?: string },
  ) => void
  onDelete: (id: number) => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [name, setName] = useState(character.name)
  const [roleLabel, setRoleLabel] = useState(character.role_label)
  const [summary, setSummary] = useState(character.summary)
  const [profile, setProfile] = useState(character.profile)
  const dirty =
    name !== character.name ||
    roleLabel !== character.role_label ||
    summary !== character.summary ||
    profile !== character.profile
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: character.id,
  })

  return (
    <li
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={isDragging ? "opacity-60" : undefined}
    >
      <div className="rounded-md border">
        <div className="flex items-center gap-2 p-2">
          <button
            type="button"
            className="cursor-grab text-muted-foreground/50 hover:text-muted-foreground disabled:cursor-not-allowed"
            aria-label={t("book:conception.drag_character")}
            disabled={!editable}
            {...attributes}
            {...listeners}
          >
            <GripVertical className="size-4" />
          </button>
          <span className="w-5 text-center text-xs tabular-nums text-muted-foreground">
            {character.rank}
          </span>
          <span className="truncate text-sm font-medium">{character.name}</span>
          {character.role_label === "" ? null : (
            <Badge variant="secondary">{character.role_label}</Badge>
          )}
          <div className="ml-auto flex items-center gap-1">
            <Button
              size="icon-xs"
              variant="ghost"
              aria-label={t("book:conception.expand_character")}
              onClick={() => setOpen((value) => !value)}
            >
              {open ? <ChevronDown /> : <ChevronRight />}
            </Button>
            {editable && character.id > 0 ? (
              <Button
                size="icon-xs"
                variant="ghost"
                aria-label={t("common:delete")}
                disabled={busy}
                onClick={() => onDelete(character.id)}
              >
                <Trash2 />
              </Button>
            ) : null}
          </div>
        </div>
        {open ? (
          <div className="space-y-2 border-t p-3">
            <div className="grid grid-cols-2 gap-2">
              <div className="space-y-1">
                <Label className="text-xs">{t("book:conception.character_name")}</Label>
                <Input
                  value={name}
                  disabled={!editable}
                  onChange={(event) => setName(event.target.value)}
                />
              </div>
              <div className="space-y-1">
                <Label className="text-xs">{t("book:conception.character_role")}</Label>
                <Input
                  value={roleLabel}
                  disabled={!editable}
                  onChange={(event) => setRoleLabel(event.target.value)}
                />
              </div>
            </div>
            <div className="space-y-1">
              <Label className="text-xs">{t("book:conception.character_summary")}</Label>
              <Input
                value={summary}
                disabled={!editable}
                onChange={(event) => setSummary(event.target.value)}
              />
            </div>
            <div className="space-y-1">
              <Label className="text-xs">{t("book:conception.character_profile")}</Label>
              <Textarea
                rows={5}
                className="font-mono text-xs"
                value={profile}
                disabled={!editable}
                onChange={(event) => setProfile(event.target.value)}
              />
            </div>
            {editable ? (
              <Button
                size="sm"
                disabled={!dirty || busy}
                onClick={() =>
                  onPatch(character.id, {
                    name: name.trim(),
                    role_label: roleLabel.trim(),
                    summary,
                    profile,
                  })
                }
              >
                {t("common:save")}
              </Button>
            ) : null}
          </div>
        ) : null}
      </div>
    </li>
  )
}
