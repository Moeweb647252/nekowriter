/**
 * The decision for a held chapter write, in the draft pane's dock (PROJECT §8.5, spec/11 §5.2,
 * D-139, D-141).
 *
 * The author's report: 「正文编写要在中间的正文草稿区进行，正文编辑时的意见提出在草稿区下方的
 * dock 栏里进行」 — the body is written in the middle draft pane, so the opinions raised while it is
 * being written belong in a dock under it, not in the agent panel on the far right. The panel could
 * only describe the staged write; the draft pane *shows* it (merged into the text, see
 * `pendingWrite.ts`), so the decision belongs where the evidence is.
 *
 * Two states, and the second one is the reason this component exists rather than a bare button pair:
 *
 * - **current**: `pending_content.expected_version` still matches the draft, so approving applies
 *   the staged content exactly. The dock says so and offers *Apply and continue*.
 * - **superseded**: the draft moved after the write was staged, and
 *   `runner::apply_pending_content` discards it — the author's text wins. The dock must not invite
 *   an "apply" that cannot happen; it says the staged text will not be applied, and its primary
 *   action is *Continue without it*, which merely resumes the session (the write is dropped in the
 *   same transaction, and the edit report tells the model what the draft now holds, spec/03 §7).
 *
 * The two mutations themselves are built by `approveHeldInput` / `requestChangesInput`, shared with
 * the agent panel (`features/agent/heldCall.ts`), so the payloads cannot drift: approve is `resume`
 * with no feedback key, request changes is `resume` with exactly the trimmed feedback, and a blank
 * request is impossible (`HeldFeedback` disables it — the server would read it as an approval).
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import type { LucideIcon } from "lucide-react"
import { Check, Pencil, TriangleAlert } from "lucide-react"

import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { Button } from "@/components/ui/button"
import { HeldFeedback } from "@/features/agent/HeldFeedback"
import { toolRenderer } from "@/features/agent/toolRenderers"
import type { PendingDraftView } from "@/features/chapter/pendingWrite"

/**
 * The tool's registry icon through a stable component type: the icon comes out of the registry at
 * render time, and React (with the compiler lint) needs a stable component for it.
 */
function DockIcon({ icon: Icon }: { icon: LucideIcon }) {
  return <Icon aria-hidden="true" className="size-4 shrink-0 text-status-waiting" />
}

export type PendingWriteDockProps = {
  /** The held write call, for the tool's own icon and one-line summary. */
  call: ToolCallDto
  /** How the staged write stands against the draft on screen. */
  view: PendingDraftView
  busy: boolean
  /** Approve: `resume` with no feedback, which applies the staged write when it is current. */
  onApprove: () => void
  /** Request changes: `resume` with exactly this feedback, which drops the staged write. */
  onRequestChanges: (feedback: string) => void
}

export function PendingWriteDock({
  call,
  view,
  busy,
  onApprove,
  onRequestChanges,
}: PendingWriteDockProps) {
  const { t } = useTranslation()
  const [changesOpen, setChangesOpen] = useState(false)
  const renderer = toolRenderer(call.name)
  const superseded = view.standing === "superseded"
  const title = superseded
    ? t("chapter:pending_title_superseded")
    : t("chapter:pending_title")
  const approveLabel = superseded
    ? t("chapter:pending_continue")
    : t("chapter:pending_approve")

  return (
    <section
      data-testid="pending-write-dock"
      data-standing={view.standing}
      aria-label={title}
      /*
        Sticky at the bottom of the draft pane: the pane scrolls a whole chapter, and a decision
        that scrolls away from the text it decides on is the problem this dock exists to fix. It
        stays inside the reading column so it reads as part of the draft, not as page chrome.
      */
      /*
        `mx-auto max-w-[42em]` matches the reading column, so the dock lines up with the text above
        it; `sticky bottom-0` keeps it in view while that text scrolls.
      */
      className="sticky bottom-0 z-20 mx-auto mt-6 max-w-[42em] space-y-2 rounded-lg border border-status-waiting/60 bg-card p-3 shadow-lg"
    >
      <div className="flex items-start gap-2">
        <DockIcon icon={renderer.icon} />
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold">{title}</h2>
          <p className="truncate text-xs text-muted-foreground">
            {renderer.summary(call.ui, t)}
          </p>
        </div>
      </div>

      {superseded ? (
        <p className="flex items-start gap-1.5 rounded bg-status-waiting/10 p-2 text-xs text-muted-foreground">
          <TriangleAlert aria-hidden="true" className="mt-0.5 size-3.5 shrink-0 text-status-waiting" />
          <span>
            {t("chapter:pending_superseded_body", {
              expected: view.expectedVersion,
              draft: view.draftVersion,
            })}
          </span>
        </p>
      ) : (
        <p className="text-xs text-muted-foreground">{t("chapter:pending_body")}</p>
      )}

      <div className="flex flex-wrap items-center gap-1">
        <Button size="sm" disabled={busy} onClick={onApprove}>
          <Check aria-hidden="true" />
          {approveLabel}
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          aria-expanded={changesOpen}
          onClick={() => setChangesOpen((open) => !open)}
        >
          <Pencil aria-hidden="true" />
          {t("chapter:hold.request_changes")}
        </Button>
      </div>

      {changesOpen ? (
        // Remounted when the disclosure reopens, so unsent feedback never reappears.
        <HeldFeedback
          key={changesOpen ? "open" : "closed"}
          disabled={busy}
          hint={t("chapter:pending_request_changes_hint")}
          onSubmit={(feedback) => {
            setChangesOpen(false)
            onRequestChanges(feedback)
          }}
        />
      ) : null}
    </section>
  )
}
