/**
 * One setting proposal (PROJECT §8.5 proposals page, spec/06 §5): the target, the rationale and the
 * computed preview diff, with Accept / Reject. A conflicting accept applies nothing and can be
 * dismissed with Reject (D-044).
 */
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Check, X } from "lucide-react"
import { useState } from "react"

import { useAcceptProposal, useRejectProposal } from "@/api/hooks/reviews"
import { useErrorToast } from "@/components/common/ErrorToast"
import { ConfirmDialog } from "@/components/common/ConfirmDialog"
import { DiffView } from "@/components/common/DiffView"
import { openChildrenFromError, type GoalOpenChild } from "@/features/canon/goalUtils"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { formatDateTime } from "@/lib/format"
import { canAccept, canReject, targetLabel } from "@/features/proposals/proposalUtils"
import type { ProposalDto } from "@/api/gen/ProposalDto"

export type ProposalCardProps = {
  proposal: ProposalDto
  bookId: number
  /** True for zh/ja/th/lo/km/my writing languages (the diff splits characters). */
  unspaced: boolean
  compact?: boolean
}

export function ProposalCard({ proposal, bookId, unspaced, compact = false }: ProposalCardProps) {
  const { t, i18n } = useTranslation()
  const showError = useErrorToast()
  const accept = useAcceptProposal(bookId)
  const reject = useRejectProposal(bookId)
  const target = targetLabel(proposal)
  const [openChildren, setOpenChildren] = useState<GoalOpenChild[]>([])

  const acceptNow = (confirmOpenChildren = false) =>
    accept.mutate(
      { proposalId: proposal.id, confirmOpenChildren },
      {
        onSuccess: (result) => {
          setOpenChildren([])
          if (result.status === "conflict") {
            toast.warning(t("book:proposal.conflict_toast"))
            return
          }
          toast.success(t("book:proposal.accepted_toast"))
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
  const onAccept = () => acceptNow(false)

  const onReject = () =>
    reject.mutate(proposal.id, {
      onSuccess: () => toast.success(t("book:proposal.rejected_toast")),
      onError: showError,
    })

  return (
    <article className={compact ? "space-y-2 rounded-md border p-3" : "space-y-3 rounded-md border p-4"}>
      <header className="flex flex-wrap items-center gap-2">
        <Badge variant="outline">{t(target.key, { id: proposal.target_id ?? 0 })}</Badge>
        {target.name === undefined ? null : (
          <span className="text-sm font-medium">{target.name}</span>
        )}
        <Badge variant={proposal.status === "conflict" ? "destructive" : "secondary"}>
          {t(`book:proposal.status.${proposal.status}`)}
        </Badge>
        <span className="ml-auto text-xs text-muted-foreground">
          {formatDateTime(proposal.decided_at ?? proposal.created_at, i18n.language)}
        </span>
      </header>
      <p className="text-xs text-muted-foreground">{proposal.rationale}</p>
      {proposal.target_type === "goal_node" && proposal.edits.kind === "set_goal_status" ? (
        <p className="text-xs">
          {t("book:proposal.goal_status_change", {
            title: proposal.goal_target?.title ?? "",
            status: t(`canon:goal_status.${proposal.edits.status}`),
          })}
        </p>
      ) : null}
      {proposal.target_type === "new_goal_task" && proposal.edits.kind === "create_goal_task" ? (
        <div className="space-y-0.5 text-xs">
          {proposal.edits.task.purpose === "" ? null : (
            <p className="text-muted-foreground">{proposal.edits.task.purpose}</p>
          )}
          {proposal.goal_target === null ? null : (
            <p className="text-muted-foreground">
              {t("book:proposal.new_goal_task_parent", { title: proposal.goal_target.title })}
            </p>
          )}
        </div>
      ) : null}
      {proposal.preview.before === "" && proposal.preview.after === "" ? null : (
        <div className="max-h-64 overflow-y-auto rounded bg-muted/40 p-2">
          <DiffView
            before={proposal.preview.before}
            after={proposal.preview.after}
            unspaced={unspaced}
            className="text-xs"
          />
        </div>
      )}
      <div className="flex flex-wrap items-center gap-2">
        {canAccept(proposal.status) ? (
          <Button size="sm" disabled={accept.isPending} onClick={onAccept}>
            <Check className="size-3.5" />
            {t("book:proposal.accept")}
          </Button>
        ) : null}
        {canReject(proposal.status) ? (
          <Button
            size="sm"
            variant="outline"
            disabled={reject.isPending}
            onClick={onReject}
          >
            <X className="size-3.5" />
            {t("book:proposal.reject")}
          </Button>
        ) : null}
      </div>
      <ConfirmDialog
        open={openChildren.length > 0}
        onOpenChange={(next) => { if (!next) setOpenChildren([]) }}
        title={t("canon:goal_dialog.open_children_title")}
        description={t("canon:goal_dialog.open_children_body")}
        confirmLabel={t("canon:goal_dialog.open_children_confirm")}
        pending={accept.isPending}
        details={
          <ul className="list-disc pl-4">
            {openChildren.map((child) => (
              <li key={child.id}>{child.title}</li>
            ))}
          </ul>
        }
        onConfirm={() => acceptNow(true)}
      />
    </article>
  )
}
