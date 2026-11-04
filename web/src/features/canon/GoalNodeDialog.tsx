/** Create or edit one goal node (spec/11 §5.5, spec/06 §11). */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Plus, Trash2 } from "lucide-react"

import type { EntityStatus } from "@/api/gen/EntityStatus"
import type { GoalCondition } from "@/api/gen/GoalCondition"
import type { GoalLevel } from "@/api/gen/GoalLevel"
import type { GoalNodeDto } from "@/api/gen/GoalNodeDto"
import type { GoalStatus } from "@/api/gen/GoalStatus"
import type { GoalNodePatch } from "@/api/endpoints/goals"
import { useCreateGoal, usePatchGoal } from "@/api/hooks/goals"
import { useEntities, useForeshadowing, useTimeline } from "@/api/hooks/useCanon"
import { useErrorToast } from "@/components/common/ErrorToast"
import { ConfirmDialog } from "@/components/common/ConfirmDialog"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { EntityPicker, EventPicker, ForeshadowingPicker } from "@/features/canon/EntityPicker"
import { conditionsOf, openChildrenFromError, type GoalOpenChild } from "@/features/canon/goalUtils"

const STATUSES: GoalStatus[] = ["active", "waiting", "done", "dropped"]
const ENTITY_STATUSES: EntityStatus[] = [
  "active",
  "inactive",
  "dead",
  "destroyed",
  "missing",
  "unknown",
]

type OpenChild = GoalOpenChild

function childLevel(parent: GoalNodeDto | undefined): GoalLevel {
  return parent?.level === "goal" ? "plan" : parent?.level === "plan" ? "task" : "goal"
}

function defaultCondition(kind: GoalCondition["kind"]): GoalCondition {
  switch (kind) {
    case "chapter_at_least":
      return { kind, chapter: 1 }
    case "foreshadowing_resolved":
      return { kind, foreshadowing_id: 0 }
    case "entity_status_is":
      return { kind, entity_id: 0, status: "active" }
    case "event_occurred":
      return { kind, event_id: 0 }
  }
}

export type GoalNodeDialogProps = {
  bookId: number
  nodes: GoalNodeDto[]
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Adding a child: the parent is fixed. */
  parentId?: number | null
  /** The selected track filter for a new top-level node. */
  trackId?: number | null
  /** Editing an existing node; absent when creating. */
  node?: GoalNodeDto | null
  onSaved?: () => void
}

export function GoalNodeDialog({
  bookId,
  nodes,
  open,
  onOpenChange,
  parentId = null,
  trackId = null,
  node = null,
  onSaved,
}: GoalNodeDialogProps) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const create = useCreateGoal(bookId)
  const patch = usePatchGoal(bookId)

  const parent = nodes.find((candidate) => candidate.id === parentId)
  const fixedLevel = node?.level ?? childLevel(parent)
  const levelEditable = node === null && parent === undefined
  const [level, setLevel] = useState<GoalLevel>(fixedLevel)
  // A child's scope is the parent's scope, even when the page's track filter is a side track.
  // Only a root creation falls back to the filter (spec/06 §11.2).
  const scopeTrack = node?.track_id ?? (parent === undefined ? trackId : parent.track_id)
  const entityChoices = useEntities(bookId, { trackId: scopeTrack ?? undefined })
  const eventChoices = useTimeline(bookId, scopeTrack ?? undefined)
  const hookChoices = useForeshadowing(bookId, undefined, scopeTrack ?? undefined)
  // The page mounts this dialog only while open, so props are the initial values; closing
  // unmounts it and the next open starts from the newly selected node.
  const [title, setTitle] = useState(node?.title ?? "")
  const [purpose, setPurpose] = useState(node?.purpose ?? "")
  const [status, setStatus] = useState<GoalStatus>(node?.status ?? "active")
  const [chapterFrom, setChapterFrom] = useState(node?.chapter_from?.toString() ?? "")
  const [chapterTo, setChapterTo] = useState(node?.chapter_to?.toString() ?? "")
  const [conditions, setConditions] = useState<GoalCondition[]>(conditionsOf(node))
  const [entityIds, setEntityIds] = useState<number[]>(node?.entities.map((link) => link.id) ?? [])
  const [eventIds, setEventIds] = useState<number[]>(node?.events.map((link) => link.id) ?? [])
  const [hookIds, setHookIds] = useState<number[]>(node?.foreshadowing.map((link) => link.id) ?? [])
  const [note, setNote] = useState("")
  const [parentChoice, setParentChoice] = useState<number | null>(node?.parent_id ?? parentId ?? null)
  const [entityPick, setEntityPick] = useState<number | undefined>(undefined)
  const [eventPick, setEventPick] = useState<number | undefined>(undefined)
  const [hookPick, setHookPick] = useState<number | undefined>(undefined)
  const [openChildren, setOpenChildren] = useState<OpenChild[]>([])

  const entityName = (id: number) =>
    node?.entities.find((link) => link.id === id)?.name ??
    entityChoices.data?.find((entity) => entity.id === id)?.name ??
    `#${id}`
  const eventName = (id: number) =>
    node?.events.find((link) => link.id === id)?.title ??
    eventChoices.data?.find((event) => event.id === id)?.title ??
    `#${id}`
  const hookName = (id: number) =>
    node?.foreshadowing.find((link) => link.id === id)?.title ??
    hookChoices.data?.find((hook) => hook.id === id)?.title ??
    `#${id}`
  const eventStatus = (id: number) =>
    node?.events.find((link) => link.id === id)?.status ??
    eventChoices.data?.find((event) => event.id === id)?.status

  const parentCandidates = nodes.filter(
    (candidate) =>
      candidate.id !== node?.id &&
      candidate.track_id === scopeTrack &&
      (level === "plan"
        ? candidate.level === "goal"
        : level === "task"
          ? candidate.level === "plan"
          : false),
  )

  const canSubmit = title.trim() !== "" && purpose.trim() !== "" && !create.isPending && !patch.isPending

  const numberOrNull = (raw: string): number | null => {
    if (raw.trim() === "") return null
    const value = Number(raw)
    return Number.isFinite(value) ? value : null
  }

  const submit = (confirmOpenChildren = false) => {
    const payload: GoalNodePatch = {
      level,
      parent_id: parentChoice,
      title: title.trim(),
      purpose: purpose.trim(),
      status,
      chapter_from: numberOrNull(chapterFrom),
      chapter_to: numberOrNull(chapterTo),
      conditions,
      entity_ids: entityIds,
      event_ids: eventIds,
      foreshadowing_ids: hookIds,
      ...(confirmOpenChildren ? { confirm_open_children: true } : {}),
    }
    if (node !== null) {
      patch.mutate(
        { id: node.id, patch: { ...payload, note } },
        {
          onSuccess: () => {
            setOpenChildren([])
            onOpenChange(false)
            onSaved?.()
          },
          onError: (error) => {
            const children = openChildrenFromError(error)
            if (children.length > 0) {
              setOpenChildren(children)
              return
            }
            showError(error)
          },
        },
      )
      return
    }
    create.mutate(
      {
        track_id: scopeTrack,
        level,
        parent_id: parentChoice,
        title: title.trim(),
        purpose: purpose.trim(),
        status,
        chapter_from: numberOrNull(chapterFrom),
        chapter_to: numberOrNull(chapterTo),
        conditions,
        entity_ids: entityIds,
        event_ids: eventIds,
        foreshadowing_ids: hookIds,
      },
      {
        onSuccess: () => {
          onOpenChange(false)
          onSaved?.()
        },
        onError: showError,
      },
    )
  }

  const updateCondition = (index: number, next: GoalCondition) => {
    setConditions((current) => current.map((value, at) => (at === index ? next : value)))
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {node === null ? t("canon:goal_dialog.create_title") : t("canon:goal_dialog.edit_title")}
          </DialogTitle>
          <DialogDescription>{t("canon:goal_dialog.description")}</DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="goal-level">{t("canon:goal_dialog.level")}</Label>
              {levelEditable ? (
                <select
                  id="goal-level"
                  value={level}
                  onChange={(event) => {
                    setLevel(event.target.value as GoalLevel)
                    setParentChoice(null)
                  }}
                  className="h-8 w-full rounded-md border bg-background px-2 text-sm"
                >
                  {(["goal", "plan", "task"] as GoalLevel[]).map((value) => (
                    <option key={value} value={value}>
                      {t(`canon:goal_level.${value}`)}
                    </option>
                  ))}
                </select>
              ) : (
                <p className="text-sm">{t(`canon:goal_level.${fixedLevel}`)}</p>
              )}
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="goal-status">{t("canon:goal_dialog.status")}</Label>
              <select
                id="goal-status"
                value={status}
                onChange={(event) => setStatus(event.target.value as GoalStatus)}
                className="h-8 w-full rounded-md border bg-background px-2 text-sm"
              >
                {STATUSES.map((value) => (
                  <option key={value} value={value}>
                    {t(`canon:goal_status.${value}`)}
                  </option>
                ))}
              </select>
            </div>
          </div>
          {level === "goal" ? null : (
            <div className="space-y-1.5">
              <Label htmlFor="goal-parent">{t("canon:goal_dialog.parent")}</Label>
              <select
                id="goal-parent"
                value={parentChoice ?? ""}
                onChange={(event) =>
                  setParentChoice(event.target.value === "" ? null : Number(event.target.value))
                }
                className="h-8 w-full rounded-md border bg-background px-2 text-sm"
              >
                <option value="">{t("canon:goal_dialog.no_parent")}</option>
                {parentCandidates.map((candidate) => (
                  <option key={candidate.id} value={candidate.id}>
                    {candidate.title}
                  </option>
                ))}
              </select>
            </div>
          )}
          <div className="space-y-1.5">
            <Label htmlFor="goal-title">{t("canon:goal_dialog.title")}</Label>
            <Input id="goal-title" value={title} onChange={(event) => setTitle(event.target.value)} />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="goal-purpose">{t("canon:goal_dialog.purpose")}</Label>
            <Textarea
              id="goal-purpose"
              rows={3}
              value={purpose}
              onChange={(event) => setPurpose(event.target.value)}
              placeholder={t("canon:goal_dialog.purpose_hint")}
            />
            <p className="text-xs text-muted-foreground">{t("canon:goal_dialog.purpose_hint")}</p>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="goal-from">{t("canon:goal_dialog.chapter_from")}</Label>
              <Input id="goal-from" type="number" value={chapterFrom} onChange={(event) => setChapterFrom(event.target.value)} />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="goal-to">{t("canon:goal_dialog.chapter_to")}</Label>
              <Input id="goal-to" type="number" value={chapterTo} onChange={(event) => setChapterTo(event.target.value)} />
            </div>
          </div>

          <section className="space-y-2">
            <div className="flex items-center justify-between">
              <Label>{t("canon:goal_dialog.conditions")}</Label>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={() => setConditions((current) => [...current, defaultCondition("chapter_at_least")])}
              >
                <Plus className="size-3.5" />
                {t("canon:goal_dialog.add_condition")}
              </Button>
            </div>
            {conditions.length === 0 ? (
              <p className="text-xs text-muted-foreground">{t("canon:goal_dialog.no_conditions")}</p>
            ) : (
              <ul className="space-y-2">
                {conditions.map((condition, index) => (
                  <li key={index} className="flex flex-wrap items-end gap-2 rounded-md border p-2">
                    <select
                      value={condition.kind}
                      aria-label={t("canon:goal_dialog.condition_kind")}
                      onChange={(event) =>
                        updateCondition(index, defaultCondition(event.target.value as GoalCondition["kind"]))
                      }
                      className="h-8 rounded-md border bg-background px-2 text-xs"
                    >
                      <option value="chapter_at_least">{t("canon:goal_condition_kind.chapter_at_least")}</option>
                      <option value="foreshadowing_resolved">{t("canon:goal_condition_kind.foreshadowing_resolved")}</option>
                      <option value="entity_status_is">{t("canon:goal_condition_kind.entity_status_is")}</option>
                      <option value="event_occurred">{t("canon:goal_condition_kind.event_occurred")}</option>
                    </select>
                    {condition.kind === "chapter_at_least" ? (
                      <Input
                        type="number"
                        aria-label={t("canon:goal_dialog.condition_chapter")}
                        value={condition.chapter}
                        onChange={(event) =>
                          updateCondition(index, {
                            kind: "chapter_at_least",
                            chapter: Number(event.target.value) || 0,
                          })
                        }
                        className="h-8 w-24"
                      />
                    ) : null}
                    {condition.kind === "foreshadowing_resolved" ? (
                      <ForeshadowingPicker
                        bookId={bookId}
                        trackId={scopeTrack ?? undefined}
                        label={t("canon:goal_dialog.condition_foreshadowing")}
                        value={condition.foreshadowing_id}
                        onChange={(value) =>
                          updateCondition(index, {
                            kind: "foreshadowing_resolved",
                            foreshadowing_id: value ?? 0,
                          })
                        }
                      />
                    ) : null}
                    {condition.kind === "event_occurred" ? (
                      <EventPicker
                        bookId={bookId}
                        trackId={scopeTrack ?? undefined}
                        label={t("canon:goal_dialog.condition_event")}
                        value={condition.event_id === 0 ? undefined : condition.event_id}
                        onChange={(value) =>
                          updateCondition(index, {
                            kind: "event_occurred",
                            event_id: value ?? 0,
                          })
                        }
                      />
                    ) : null}
                    {condition.kind === "entity_status_is" ? (
                      <>
                        <EntityPicker
                          bookId={bookId}
                          trackId={scopeTrack ?? undefined}
                          label={t("canon:goal_dialog.condition_entity")}
                          value={condition.entity_id}
                          onChange={(value) =>
                            updateCondition(index, {
                              kind: "entity_status_is",
                              entity_id: value ?? 0,
                              status: condition.status,
                            })
                          }
                        />
                        <select
                          value={condition.status}
                          aria-label={t("canon:goal_dialog.condition_status")}
                          onChange={(event) =>
                            updateCondition(index, {
                              kind: "entity_status_is",
                              entity_id: condition.entity_id,
                              status: event.target.value as EntityStatus,
                            })
                          }
                          className="h-8 rounded-md border bg-background px-2 text-xs"
                        >
                          {ENTITY_STATUSES.map((value) => (
                            <option key={value} value={value}>
                              {t(`canon:status.${value}`)}
                            </option>
                          ))}
                        </select>
                      </>
                    ) : null}
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      aria-label={t("canon:goal_dialog.remove_condition")}
                      onClick={() => setConditions((current) => current.filter((_, at) => at !== index))}
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </li>
                ))}
              </ul>
            )}
          </section>

          <section className="space-y-2">
            <Label>{t("canon:goal_dialog.links")}</Label>
            <div className="flex flex-wrap items-center gap-2">
              <EntityPicker
                bookId={bookId}
                trackId={scopeTrack ?? undefined}
                label={t("canon:goal_dialog.link_entity")}
                value={entityPick}
                onChange={setEntityPick}
              />
              <Button type="button" size="sm" variant="outline" disabled={entityPick === undefined} onClick={() => { if (entityPick !== undefined) { setEntityIds((current) => current.includes(entityPick) ? current : [...current, entityPick]); setEntityPick(undefined) } }}>
                <Plus className="size-3.5" />{t("canon:goal_dialog.add_link")}
              </Button>
            </div>
            <div className="flex flex-wrap gap-1">
              {entityIds.map((id) => (
                <span key={id} className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-xs">
                  {entityName(id)}
                  <button type="button" aria-label={t("canon:goal_dialog.remove_link")} onClick={() => setEntityIds((current) => current.filter((item) => item !== id))}>
                    ×
                  </button>
                </span>
              ))}
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <EventPicker
                bookId={bookId}
                trackId={scopeTrack ?? undefined}
                label={t("canon:goal_dialog.link_event")}
                value={eventPick}
                onChange={setEventPick}
              />
              <Button type="button" size="sm" variant="outline" disabled={eventPick === undefined} onClick={() => { if (eventPick !== undefined) { setEventIds((current) => current.includes(eventPick) ? current : [...current, eventPick]); setEventPick(undefined) } }}>
                <Plus className="size-3.5" />{t("canon:goal_dialog.add_link")}
              </Button>
            </div>
            <div className="flex flex-wrap gap-1">
              {eventIds.map((id) => (
                <span key={id} className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-xs">
                  {eventName(id)}
                  {eventStatus(id) === "planned" ? (
                    <span className="rounded border border-dashed px-1 text-[0.6rem]">
                      {t("canon:planned_badge")}
                    </span>
                  ) : null}
                  <button type="button" aria-label={t("canon:goal_dialog.remove_link")} onClick={() => setEventIds((current) => current.filter((item) => item !== id))}>
                    ×
                  </button>
                </span>
              ))}
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <ForeshadowingPicker
                bookId={bookId}
                trackId={scopeTrack ?? undefined}
                label={t("canon:goal_dialog.link_foreshadowing")}
                value={hookPick}
                onChange={setHookPick}
              />
              <Button type="button" size="sm" variant="outline" disabled={hookPick === undefined} onClick={() => { if (hookPick !== undefined) { setHookIds((current) => current.includes(hookPick) ? current : [...current, hookPick]); setHookPick(undefined) } }}>
                <Plus className="size-3.5" />{t("canon:goal_dialog.add_link")}
              </Button>
            </div>
            <div className="flex flex-wrap gap-1">
              {hookIds.map((id) => (
                <span key={id} className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-xs">
                  {hookName(id)}
                  <button type="button" aria-label={t("canon:goal_dialog.remove_link")} onClick={() => setHookIds((current) => current.filter((item) => item !== id))}>
                    ×
                  </button>
                </span>
              ))}
            </div>
          </section>

          {node === null ? null : (
            <div className="space-y-1.5">
              <Label htmlFor="goal-note">{t("canon:goal_dialog.change_note")}</Label>
              <Textarea id="goal-note" rows={2} value={note} onChange={(event) => setNote(event.target.value)} />
            </div>
          )}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>{t("common:cancel")}</Button>
          <Button type="button" disabled={!canSubmit} onClick={() => submit(false)}>{t("common:save")}</Button>
        </DialogFooter>
      </DialogContent>
      <ConfirmDialog
        open={openChildren.length > 0}
        onOpenChange={(next) => { if (!next) setOpenChildren([]) }}
        title={t("canon:goal_dialog.open_children_title")}
        description={t("canon:goal_dialog.open_children_body")}
        confirmLabel={t("canon:goal_dialog.open_children_confirm")}
        pending={patch.isPending}
        details={<ul className="list-disc pl-4">{openChildren.map((child) => <li key={child.id}>{child.title}</li>)}</ul>}
        onConfirm={() => submit(true)}
      />
    </Dialog>
  )
}

export default GoalNodeDialog
