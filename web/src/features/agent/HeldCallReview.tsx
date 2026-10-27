/**
 * The held-call review (PROJECT §8.6, spec/11 §5.3): the panel's `held` counterpart of the
 * conception studio's `awaiting_input` forms.
 *
 * A `held` session is not a pause without content: the runner stopped *on a tool call* whose
 * deferred effects wait for the author (spec/03 §5.2 `hold_condition`, §5.4). The generic
 * Continue / Feedback… buttons were the same two calls without saying which tool, what it
 * submitted, or that Continue means "approve this outline" — the author reported exactly that
 * (「章节细纲等待我的审阅，但是侧边栏没有审阅能力」). This block therefore shows, in one fixed
 * place next to the controls:
 *
 * - **what** is held, through the tool renderer registry the tool cards already use
 *   (`agent:tools.<name>`, so no second naming scheme and no raw tool identifier), and
 * - the submitted text itself, so an approval is never given to text the author cannot read, and
 * - the two decisions in the domain's words: **Approve** (resume with no feedback) and
 *   **Request changes** (resume with that feedback — the server revokes the deferred effects and
 *   appends the revoke notice plus the feedback, spec/03 §5.4).
 *
 * The content is rendered here rather than linked to the tool card: the stream is virtualized (a
 * row scrolled out of view is not in the DOM), the card is collapsed by default and shows the raw
 * JSON input, and the decision is made at the controls. The block is bounded and scrolls on its
 * own, so a 4,000-character outline cannot push the stream out of the panel.
 *
 * **This block does not decide a held chapter write** (D-141). A write tool's hold is about the
 * draft itself, so it is decided in the draft pane's dock, next to the merged text it would
 * produce; the panel's `SessionControls` keeps the generic controls and the panel points at the
 * dock instead of showing a second, differently-worded decision surface here. Everything else —
 * including every non-write hold — still comes through this block, unchanged.
 *
 * The pure parts (which call is held, what text it carries, which wording it takes, and the two
 * mutation payloads) live in `heldCall.ts`, because the dock uses the same ones.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import type { LucideIcon } from "lucide-react"
import ReactMarkdown from "react-markdown"
import remarkGfm from "remark-gfm"
import { Check, Pencil } from "lucide-react"

import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { Button } from "@/components/ui/button"
import { HeldFeedback } from "@/features/agent/HeldFeedback"
import { heldBodyText, heldReferenceNotes, heldReviewCase } from "@/features/agent/heldCall"
import { toolRenderer } from "@/features/agent/toolRenderers"

// Re-exported so the existing call sites and tests keep one import site for the held-call logic,
// while the definitions live in the pure module both surfaces share (D-141).
export {
  findHeldCall,
  heldBodyText,
  heldReferenceNotes,
  heldReviewCase,
  type HeldReviewCase,
} from "@/features/agent/heldCall"

/**
 * The renderer's icon through a stable component type: the icon comes out of the registry at
 * render time, and React (with the compiler lint) needs a stable component for it.
 */
function ReviewIcon({ icon: Icon }: { icon: LucideIcon }) {
  return <Icon aria-hidden="true" className="size-4 shrink-0 text-amber-600 dark:text-amber-400" />
}

export function HeldCallReview({
  call,
  busy,
  onApprove,
  onRequestChanges,
}: {
  call: ToolCallDto
  busy: boolean
  /** Approve: `resume` with no feedback (the held call is finalized with its effects). */
  onApprove: () => void
  /** Request changes: `resume` with exactly this feedback (the effects are revoked). */
  onRequestChanges: (feedback: string) => void
}) {
  const { t } = useTranslation()
  // The feedback box is a disclosure: closing it drops the draft, so a later open starts clean
  // (`key` remounts `HeldFeedback`, whose text is its own state).
  const [changesOpen, setChangesOpen] = useState(false)

  const kind = heldReviewCase(call.name)
  const renderer = toolRenderer(call.name)
  const title =
    kind === "outline"
      ? t("agent:held.outline_title")
      : kind === "revision_plan"
        ? t("agent:held.revision_plan_title")
        : t("agent:held.generic_title")
  const approveLabel =
    kind === "outline"
      ? t("agent:held.approve_outline")
      : kind === "revision_plan"
        ? t("agent:held.approve_revision_plan")
        : t("agent:held.approve_generic")
  const changesHint =
    kind === "outline"
      ? t("agent:held.request_changes_hint_outline")
      : kind === "revision_plan"
        ? t("agent:held.request_changes_hint_revision_plan")
        : t("agent:held.request_changes_hint")
  const body = heldBodyText(call)
  const notes = heldReferenceNotes(call)

  // `HeldFeedback` only calls this with trimmed, non-empty text — the server reads blank feedback
  // as "no feedback" and would *approve* the call instead (`services/sessions.rs` `resume_inner`).
  const submit = (feedback: string) => {
    setChangesOpen(false)
    onRequestChanges(feedback)
  }

  return (
    <section
      data-testid="held-review"
      data-held-case={kind}
      aria-label={title}
      /*
        The decisions sit in the header and the submitted text scrolls under them, the way the
        studio's pending-form block is bounded (spec/11 §5.4). An outline runs to thousands of
        characters: with no bound it pushed the activity stream out of the panel entirely, and with
        the actions at the bottom they could be scrolled away from the text being decided on.
      */
      className="shrink-0 space-y-2 border-b bg-amber-500/5 px-3 py-2"
    >
      <div className="flex items-start gap-2">
        <ReviewIcon icon={renderer.icon} />
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold">{title}</h2>
          {/* The tool's own one-line summary (spec/11 §5.3), so the block names the same thing
              the card in the stream does. */}
          <p className="truncate text-xs text-muted-foreground">{renderer.summary(call.ui, t)}</p>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-1">
        <Button size="sm" disabled={busy} onClick={() => onApprove()}>
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
          {t("agent:held.request_changes")}
        </Button>
      </div>

      {changesOpen ? (
        // Remounted whenever the disclosure reopens, so unsent text never reappears.
        <HeldFeedback
          key={changesOpen ? "open" : "closed"}
          disabled={busy}
          hint={changesHint}
          onSubmit={submit}
        />
      ) : null}

      {body === "" ? null : (
        <div className="prose prose-sm max-h-32 max-w-none overflow-auto rounded border bg-card p-2 text-xs whitespace-pre-wrap dark:prose-invert">
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{body}</ReactMarkdown>
        </div>
      )}

      {notes === "" ? null : (
        <details className="rounded border bg-muted/30 p-2 text-xs">
          <summary className="cursor-pointer text-muted-foreground">
            {t("chapter:reference_notes")}
          </summary>
          <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap text-muted-foreground">
            {notes}
          </pre>
        </details>
      )}
    </section>
  )
}
