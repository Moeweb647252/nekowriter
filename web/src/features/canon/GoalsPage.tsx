/** The Goals page (spec/11 §5.5, spec/06 §11): tree, readiness, conditions, links and proposals. */
import { useMemo, useState } from "react"
import { useParams, useSearchParams } from "react-router"
import { useTranslation } from "react-i18next"
import {
  AlertTriangle,
  Ban,
  Check,
  CheckCircle2,
  Circle,
  Flag,
  MoreHorizontal,
  Pencil,
  Plus,
  Power,
  Route,
  Target,
  Trash2,
} from "lucide-react"

import type { GoalNodeDto } from "@/api/gen/GoalNodeDto"
import type { ProposalDto } from "@/api/gen/ProposalDto"
import { useProposals } from "@/api/hooks/reviews"
import { useChapters, useTracks } from "@/api/hooks/tracks"
import { useDeleteGoal, useGoals, usePatchGoal } from "@/api/hooks/goals"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { EmptyState } from "@/components/common/EmptyState"
import { ConfirmDialog } from "@/components/common/ConfirmDialog"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Skeleton } from "@/components/ui/skeleton"
import { GoalNodeDialog } from "@/features/canon/GoalNodeDialog"
import {
  conditionSentence,
  hasClosedAncestor,
  openChildrenFromError,
  pathLabel,
  type GoalOpenChild,
} from "@/features/canon/goalUtils"
import { ProposalCard } from "@/features/proposals/ProposalCard"
import { cn } from "@/lib/utils"

type DialogState = {
  parentId: number | null
  trackId: number | null
  node: GoalNodeDto | null
}

export default function GoalsPage() {
  const { t } = useTranslation()
  const { bookId: rawBookId } = useParams()
  const bookId = Number(rawBookId)
  const [searchParams] = useSearchParams()
  const focusedNodeId = Number(searchParams.get("node"))
  const [trackId, setTrackId] = useState<number | undefined>(undefined)
  const [readyOnly, setReadyOnly] = useState(searchParams.get("filter") === "ready")
  const [dialog, setDialog] = useState<DialogState | null>(null)
  const goals = useGoals(bookId, { track_id: trackId })
  const tracks = useTracks(bookId)
  const mainTrack = (tracks.data ?? []).find((track) => track.kind === "main")
  const mainTrackId = mainTrack?.id ?? 0
  const proposals = useProposals(bookId, "pending")
  const patch = usePatchGoal(bookId)
  const remove = useDeleteGoal(bookId)
  const showError = useErrorToast()
  const [pendingStatus, setPendingStatus] = useState<{
    node: GoalNodeDto
    status: GoalNodeDto["status"]
  } | null>(null)
  const [openChildren, setOpenChildren] = useState<GoalOpenChild[]>([])
  const nodes = goals.data ?? []
  const isolatedTracks = (tracks.data ?? []).filter(
    (track) => track.kind === "side" && track.canon_mode === "isolated",
  )
  const visibleNodes = nodes.filter((node) => node.status !== "dropped")
  const readyNodes = visibleNodes.filter((node) => node.ready || node.closable)
  const droppedNodes = nodes.filter((node) => node.status === "dropped")

  const pendingProposals = useMemo(
    () => (proposals.data ?? []).filter((proposal) => proposal.status === "pending"),
    [proposals.data],
  )

  if (goals.isPending) {
    return <Skeleton className="m-6 h-64" />
  }

  const setStatus = (node: GoalNodeDto, status: GoalNodeDto["status"]) => {
    patch.mutate(
      { id: node.id, patch: { status } },
      {
        onError: (error) => {
          const children = openChildrenFromError(error)
          if (children.length > 0) {
            setPendingStatus({ node, status })
            setOpenChildren(children)
            return
          }
          showError(error)
        },
      },
    )
  }

  const markDone = (node: GoalNodeDto) => setStatus(node, "done")

  const openCreate = (parentId: number | null = null) =>
    setDialog({ parentId, trackId: trackId ?? null, node: null })
  const openEdit = (node: GoalNodeDto) =>
    setDialog({ parentId: node.parent_id, trackId: node.track_id, node })

  const renderRow = (
    node: GoalNodeDto,
    depth: number,
    path?: string,
    includeChildren = true,
  ) => {
    const row = (
      <GoalRow
        key={node.id}
        node={node}
        depth={depth}
        path={path}
        nodes={nodes}
        mainTrackId={mainTrackId}
        onEdit={() => openEdit(node)}
        onAddChild={node.level === "task" ? undefined : () => openCreate(node.id)}
        onActivate={() => setStatus(node, "active")}
        onMarkDone={() => markDone(node)}
        onDrop={() => setStatus(node, "dropped")}
        onDelete={() => remove.mutate(node.id, { onError: showError })}
        focused={focusedNodeId === node.id}
      />
    )
    const statusProposal = pendingProposals.find(
      (proposal) =>
        proposal.target_type === "goal_node" && proposal.goal_target?.id === node.id,
    )
    const children = includeChildren
      ? nodes
          .filter((candidate) => candidate.parent_id === node.id && candidate.status !== "dropped")
          .sort((left, right) => left.id - right.id)
      : []
    const childProposals = includeChildren
      ? pendingProposals.filter(
          (proposal) =>
            proposal.target_type === "new_goal_task" &&
            proposal.edits.kind === "create_goal_task" &&
            proposal.edits.task.parent_id === node.id,
        )
      : []
    return (
      <li key={node.id}>
        {row}
        {statusProposal === undefined ? null : (
          <InlineProposal proposal={statusProposal} bookId={bookId} />
        )}
        {children.length === 0 && childProposals.length === 0 ? null : (
          <ul className="ml-5 border-l pl-3">
            {childProposals.map((proposal) => (
              <li key={`proposal-${proposal.id}`}>
                <InlineProposal proposal={proposal} bookId={bookId} />
              </li>
            ))}
            {children.map((child) => renderRow(child, depth + 1))}
          </ul>
        )}
      </li>
    )
  }

  const roots = visibleNodes.filter(
    (node) => node.parent_id === null || !visibleNodes.some((candidate) => candidate.id === node.parent_id),
  )
  const rootProposals = pendingProposals.filter(
    (proposal) =>
      proposal.target_type === "new_goal_task" &&
      proposal.edits.kind === "create_goal_task" &&
      proposal.edits.task.parent_id === null,
  )

  return (
    <div className="mx-auto max-w-4xl space-y-4 p-6">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold">{t("canon:goals")}</h1>
          <p className="text-xs text-muted-foreground">{t("canon:goals_hint")}</p>
        </div>
        <Button size="sm" onClick={() => openCreate(null)}>
          <Plus className="size-3.5" />
          {t("canon:goal_add")}
        </Button>
      </header>

      <div className="flex flex-wrap items-center gap-2">
        {isolatedTracks.length === 0 ? null : (
          <select
            value={trackId ?? ""}
            onChange={(event) =>
              setTrackId(event.target.value === "" ? undefined : Number(event.target.value))
            }
            className="h-8 rounded-md border bg-background px-2 text-xs"
            aria-label={t("canon:goal_track_filter")}
          >
            <option value="">{t("canon:goal_shared")}</option>
            {isolatedTracks.map((track) => (
              <option key={track.id} value={track.id}>
                {track.title}
              </option>
            ))}
          </select>
        )}
        <Button
          size="sm"
          variant={readyOnly ? "default" : "outline"}
          onClick={() => setReadyOnly((value) => !value)}
        >
          <CheckCircle2 className="size-3.5" />
          {t("canon:goal_ready_filter")}
          {readyNodes.length === 0 ? null : <span className="ml-1 text-xs">{readyNodes.length}</span>}
        </Button>
      </div>

      {visibleNodes.length === 0 && droppedNodes.length === 0 && pendingProposals.length === 0 ? (
        <EmptyState title={t("canon:no_goals")} description={t("canon:no_goals_hint")} />
      ) : readyOnly ? (
        readyNodes.length === 0 ? (
          <EmptyState title={t("canon:no_ready_goals")} description={t("canon:no_ready_goals_hint")} />
        ) : (
          <ul className="space-y-2">
            {readyNodes.map((node) => renderRow(node, 0, pathLabel(nodes, node.id), false))}
          </ul>
        )
      ) : (
        <ul className="space-y-1">
          {rootProposals.map((proposal) => (
            <li key={`proposal-${proposal.id}`}>
              <InlineProposal proposal={proposal} bookId={bookId} />
            </li>
          ))}
          {roots.map((node) => renderRow(node, 0))}
        </ul>
      )}

      {droppedNodes.length === 0 ? null : (
        <details className="rounded-md border p-2">
          <summary className="cursor-pointer text-xs text-muted-foreground">
            {t("canon:goal_dropped", { count: droppedNodes.length })}
          </summary>
          <ul className="mt-2 space-y-1">
            {droppedNodes.map((node) => (
              <li key={node.id} className="flex items-center gap-2 px-2 py-1 text-sm opacity-60">
                <GoalLevelIcon level={node.level} />
                <span className="line-through">{node.title}</span>
              </li>
            ))}
          </ul>
        </details>
      )}

      <ConfirmDialog
        open={pendingStatus !== null && openChildren.length > 0}
        onOpenChange={(next) => {
          if (!next) {
            setPendingStatus(null)
            setOpenChildren([])
          }
        }}
        title={t("canon:goal_dialog.open_children_title")}
        description={t("canon:goal_dialog.open_children_body")}
        confirmLabel={t("canon:goal_dialog.open_children_confirm")}
        pending={patch.isPending}
        details={
          <ul className="list-disc pl-4">
            {openChildren.map((child) => (
              <li key={child.id}>{child.title}</li>
            ))}
          </ul>
        }
        onConfirm={() => {
          if (pendingStatus === null) return
          patch.mutate(
            {
              id: pendingStatus.node.id,
              patch: {
                status: pendingStatus.status,
                confirm_open_children: true,
              },
            },
            {
              onSuccess: () => {
                setPendingStatus(null)
                setOpenChildren([])
              },
              onError: showError,
            },
          )
        }}
      />

      {dialog === null ? null : (
        <GoalNodeDialog
          key={dialog.node?.id ?? `new-${dialog.parentId ?? "root"}-${dialog.trackId ?? "shared"}`}
          bookId={bookId}
          nodes={nodes}
          open
          onOpenChange={(open) => {
            if (!open) setDialog(null)
          }}
          parentId={dialog.parentId}
          trackId={dialog.trackId}
          node={dialog.node}
        />
      )}
    </div>
  )
}

function InlineProposal({ proposal, bookId }: { proposal: ProposalDto; bookId: number }) {
  return <ProposalCard proposal={proposal} bookId={bookId} unspaced={false} compact />
}

/** One goal tree row. */
function GoalRow({
  node,
  depth,
  path,
  nodes,
  mainTrackId,
  onEdit,
  onAddChild,
  onActivate,
  onMarkDone,
  onDrop,
  onDelete,
  focused = false,
}: {
  node: GoalNodeDto
  depth: number
  path?: string
  nodes: GoalNodeDto[]
  mainTrackId: number
  onEdit: () => void
  onAddChild?: () => void
  onActivate: () => void
  onMarkDone: () => void
  onDrop: () => void
  onDelete: () => void
  focused?: boolean
}) {
  const { t } = useTranslation()
  const trackChapters = useChapters(node.track_id ?? mainTrackId)
  const latestOfficial = (trackChapters.data ?? [])
    .filter((chapter) => chapter.status === "official")
    .reduce((latest, chapter) => Math.max(latest, chapter.number), 0)
  const closedAncestor = hasClosedAncestor(nodes, node)
  return (
    <div
      className={cn(
        "rounded-md border p-2",
        focused && "ring-2 ring-primary/40",
        node.status === "done" && "opacity-60",
        depth > 0 && "ml-0",
      )}
    >
      <div className="flex flex-wrap items-start gap-2">
        <GoalLevelIcon level={node.level} />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium">{node.title}</span>
            <Badge variant="secondary">{t(`canon:goal_status.${node.status}`)}</Badge>
            {node.ready ? <Badge>{t("canon:goal_ready")}</Badge> : null}
            {node.track_id === null ? null : (
              <Badge variant="outline">{t("canon:private_badge")}</Badge>
            )}
            {path === undefined ? null : (
              <span className="text-[0.65rem] text-muted-foreground">{path}</span>
            )}
          </div>
          <p className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">{node.purpose}</p>
          <div className="mt-1 flex flex-wrap items-center gap-1.5 text-xs">
            {rangeLabel(t, node) === null ? null : (
              <span
                className={cn(
                  "rounded bg-muted px-1.5 py-0.5 text-[0.65rem] text-muted-foreground",
                  node.chapter_to !== null && latestOfficial > node.chapter_to && "bg-destructive/10 text-destructive",
                )}
              >
                {rangeLabel(t, node)}
              </span>
            )}
            {node.entities.map((link) => (
              <span key={`entity-${link.id}`} className="rounded-full bg-muted px-1.5 py-0.5 text-[0.65rem]">
                {link.name}
              </span>
            ))}
            {node.events.map((link) => (
              <span key={`event-${link.id}`} className="rounded-full bg-muted px-1.5 py-0.5 text-[0.65rem]">
                {link.title}
              </span>
            ))}
            {node.foreshadowing.map((link) => (
              <span key={`hook-${link.id}`} className="rounded-full bg-muted px-1.5 py-0.5 text-[0.65rem]">
                {link.title}
              </span>
            ))}
          </div>
          {node.status !== "waiting" ? null : (
            <ul className="mt-1 space-y-0.5 text-[0.7rem]">
              {node.conditions.map((condition, index) => (
                <li key={index} className={cn("flex items-center gap-1", condition.broken && "text-destructive")}>
                  {condition.broken ? (
                    <AlertTriangle
                      className="size-3 shrink-0"
                      aria-label={t("canon:goal_condition.broken")}
                    />
                  ) : condition.satisfied ? (
                    <CheckCircle2
                      className="size-3 shrink-0 text-status-done"
                      aria-label={t("canon:goal_condition.satisfied")}
                    />
                  ) : (
                    <Circle
                      className="size-3 shrink-0 text-muted-foreground"
                      aria-label={t("canon:goal_condition.open")}
                    />
                  )}
                  <span>{conditionSentence(t, condition)}</span>
                  {condition.broken ? <span> · {t("canon:goal_condition.broken_hint")}</span> : null}
                </li>
              ))}
            </ul>
          )}
          {node.closable ? (
            <div className="mt-1 flex items-center gap-2 text-xs text-muted-foreground">
              <span>{t("canon:goal_closable")}</span>
              <Button size="sm" variant="outline" onClick={onMarkDone}>
                {t("canon:goal_mark_done")}
              </Button>
            </div>
          ) : null}
          {closedAncestor ? (
            <p className="mt-1 text-[0.65rem] text-muted-foreground">{t("canon:goal_not_sent")}</p>
          ) : null}
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button size="sm" variant="ghost" aria-label={t("canon:goal_actions", { title: node.title })}>
              <MoreHorizontal className="size-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={onEdit}>
              <Pencil className="size-3.5" />
              {t("canon:goal_edit")}
            </DropdownMenuItem>
            {onAddChild === undefined ? null : (
              <DropdownMenuItem onSelect={onAddChild}>
                <Plus className="size-3.5" />
                {t("canon:goal_add_child")}
              </DropdownMenuItem>
            )}
            <DropdownMenuItem onSelect={onActivate}>
              <Power className="size-3.5" />
              {t("canon:goal_activate")}
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={onMarkDone}>
              <Check className="size-3.5" />
              {t("canon:goal_mark_done")}
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={onDrop}>
              <Ban className="size-3.5" />
              {t("canon:goal_drop")}
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem variant="destructive" onSelect={onDelete}>
              <Trash2 className="size-3.5" />
              {t("canon:goal_delete")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  )
}

function GoalLevelIcon({ level }: { level: GoalNodeDto["level"] }) {
  if (level === "goal") return <Target className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
  if (level === "plan") return <Route className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
  return <Flag className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
}

function rangeLabel(
  t: (key: string, options?: Record<string, unknown>) => string,
  node: GoalNodeDto,
): string | null {
  if (node.chapter_from === null && node.chapter_to === null) return null
  if (node.chapter_from !== null && node.chapter_to !== null) {
    return t("canon:goal_range.between", { from: node.chapter_from, to: node.chapter_to })
  }
  if (node.chapter_from !== null) {
    return t("canon:goal_range.from", { from: node.chapter_from })
  }
  return t("canon:goal_range.to", { to: node.chapter_to })
}
