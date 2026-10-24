/** The ranked character list with drag and drop (PROJECT §8.9, spec/11 §5.4). */
import { useState } from "react"
import { useParams } from "react-router"
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
import { ChevronDown, ChevronRight, GripVertical, Plus, Trash2 } from "lucide-react"

import {
  useCharacters,
  useCreateCharacter,
  useDeleteCharacter,
  usePatchCharacter,
  useReorderCharacters,
} from "@/api/hooks/settings-materials"
import { useErrorToast } from "@/components/common/ErrorToast"
import { ConfirmDialog } from "@/components/common/ConfirmDialog"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Skeleton } from "@/components/ui/skeleton"
import { Textarea } from "@/components/ui/textarea"
import { Badge } from "@/components/ui/badge"
import type { CharacterDto } from "@/api/gen/CharacterDto"
import ReactMarkdown from "react-markdown"
import remarkGfm from "remark-gfm"

export default function CharactersPage() {
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const { t } = useTranslation()
  const showError = useErrorToast()
  const characters = useCharacters(bookId)
  const create = useCreateCharacter(bookId)
  const reorder = useReorderCharacters(bookId)
  const [newName, setNewName] = useState("")
  const [newRole, setNewRole] = useState("")

  const list = characters.data ?? []
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )

  const onDragEnd = (event: DragEndEvent) => {
    const { active, over } = event
    if (over === null || active.id === over.id) {
      return
    }
    const oldIndex = list.findIndex((character) => character.id === active.id)
    const newIndex = list.findIndex((character) => character.id === over.id)
    if (oldIndex < 0 || newIndex < 0) {
      return
    }
    reorder.mutate(arrayMove(list, oldIndex, newIndex).map((character) => character.id), {
      onError: showError,
    })
  }

  return (
    <div className="mx-auto max-w-3xl space-y-4 p-6">
      <header>
        <h1 className="text-xl font-semibold">{t("book:characters")}</h1>
        <p className="mt-1 text-xs text-muted-foreground">{t("book:characters_hint")}</p>
      </header>

      <div className="flex flex-wrap items-end gap-2 rounded-md border p-3">
        <div className="flex-1 space-y-1">
          <Label htmlFor="new-character-name" className="text-xs">
            {t("book:character_name")}
          </Label>
          <Input
            id="new-character-name"
            value={newName}
            onChange={(event) => setNewName(event.target.value)}
          />
        </div>
        <div className="flex-1 space-y-1">
          <Label htmlFor="new-character-role" className="text-xs">
            {t("book:character_role_label")}
          </Label>
          <Input
            id="new-character-role"
            value={newRole}
            onChange={(event) => setNewRole(event.target.value)}
          />
        </div>
        <Button
          size="sm"
          disabled={newName.trim() === "" || create.isPending}
          onClick={() =>
            create.mutate(
              { name: newName.trim(), role_label: newRole.trim(), summary: "", profile: "", note: "" },
              {
                onSuccess: () => {
                  setNewName("")
                  setNewRole("")
                },
                onError: showError,
              },
            )
          }
        >
          <Plus />
          {t("common:create")}
        </Button>
      </div>

      {characters.isPending ? (
        <Skeleton className="h-32" />
      ) : list.length === 0 ? (
        <EmptyState title={t("book:no_characters")} description={t("book:no_characters_hint")} />
      ) : (
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
          <SortableContext
            items={list.map((character) => character.id)}
            strategy={verticalListSortingStrategy}
          >
            <ul className="space-y-2">
              {list.map((character) => (
                <CharacterRow key={character.id} bookId={bookId} character={character} />
              ))}
            </ul>
          </SortableContext>
        </DndContext>
      )}
    </div>
  )
}

/** One ranked character: rank, name, role label, editable summary/profile, delete. */
function CharacterRow({ bookId, character }: { bookId: number; character: CharacterDto }) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const patch = usePatchCharacter(bookId)
  const remove = useDeleteCharacter(bookId)
  const [expanded, setExpanded] = useState(false)
  const [summary, setSummary] = useState(character.summary)
  const [profile, setProfile] = useState(character.profile)
  const [confirming, setConfirming] = useState(false)
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
            className="cursor-grab text-muted-foreground/50 hover:text-muted-foreground"
            aria-label={t("book:drag_character")}
            {...attributes}
            {...listeners}
          >
            <GripVertical className="size-4" />
          </button>
          <span className="w-5 text-center text-xs tabular-nums text-muted-foreground">
            {character.rank}
          </span>
          <span className="font-medium">{character.name}</span>
          {character.role_label === "" ? null : (
            <Badge variant="secondary">{character.role_label}</Badge>
          )}
          {character.entity_id === null ? null : (
            <Badge variant="outline">#{character.entity_id}</Badge>
          )}
          <div className="ml-auto flex items-center gap-1">
            <Button
              size="icon-xs"
              variant="ghost"
              aria-label={t("book:expand_character")}
              onClick={() => setExpanded((value) => !value)}
            >
              {expanded ? <ChevronDown /> : <ChevronRight />}
            </Button>
            <Button
              size="icon-xs"
              variant="ghost"
              aria-label={t("common:delete")}
              onClick={() => setConfirming(true)}
            >
              <Trash2 />
            </Button>
          </div>
        </div>
        {expanded ? (
          <div className="space-y-2 border-t p-3">
            <div className="space-y-1">
              <Label className="text-xs">{t("book:character_summary")}</Label>
              <Input value={summary} onChange={(event) => setSummary(event.target.value)} />
            </div>
            <div className="space-y-1">
              <Label className="text-xs">{t("book:character_profile")}</Label>
              <Textarea
                value={profile}
                rows={6}
                className="font-mono text-xs"
                onChange={(event) => setProfile(event.target.value)}
              />
            </div>
            {profile.trim() === "" ? null : (
              <div className="prose-latin max-w-none space-y-2 rounded-md bg-muted/40 p-3">
                <ReactMarkdown remarkPlugins={[remarkGfm]}>{profile}</ReactMarkdown>
              </div>
            )}
            <Button
              size="sm"
              disabled={
                patch.isPending ||
                (summary === character.summary && profile === character.profile)
              }
              onClick={() =>
                patch.mutate(
                  { id: character.id, patch: { summary, profile, note: "" } },
                  { onError: showError },
                )
              }
            >
              {patch.isPending ? t("common:saving") : t("common:save")}
            </Button>
          </div>
        ) : null}
      </div>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("book:delete_character_title", { name: character.name })}
        description={t("book:delete_character_body")}
        pending={remove.isPending}
        onConfirm={() =>
          remove.mutate(character.id, {
            onSuccess: () => setConfirming(false),
            onError: showError,
          })
        }
      />
    </li>
  )
}
