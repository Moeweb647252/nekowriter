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
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import type { LucideIcon } from "lucide-react"
import ReactMarkdown from "react-markdown"
import remarkGfm from "remark-gfm"
import { Check, Pencil } from "lucide-react"

import type { MessageWithCallsDto } from "@/api/gen/MessageWithCallsDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { toolRenderer } from "@/features/agent/toolRenderers"
import { isComposing } from "@/lib/hotkeys"

/** The three wordings a held call can take. */
export type HeldReviewCase = "outline" | "revision_plan" | "generic"

/**
 * Which wording a held tool takes.
 *
 * `submit_outline` and `submit_revision_plan` are the holds the runtime designs for review
 * (spec/03 §5.4 names both); every other held call comes from `pause_requested` or `auto_pause`,
 * where the tool name varies and the generic wording is the honest one.
 */
export function heldReviewCase(name: string): HeldReviewCase {
  switch (name) {
    case "submit_outline":
      return "outline"
    case "submit_revision_plan":
      return "revision_plan"
    default:
      return "generic"
  }
}

/**
 * The input field that carries the text under review, per tool.
 *
 * These mirror the tools' input schemas (spec/05 §4); a tool without an entry falls back to the
 * first non-empty field of [`FALLBACK_FIELDS`].
 */
const BODY_FIELDS: Record<string, string> = {
  submit_outline: "outline",
  submit_revision_plan: "plan",
  insert_text: "text",
  insert_paragraphs: "text",
  rewrite_paragraph: "text",
  replace_text: "new",
  web_fetch: "url",
  web_search: "query",
}

/** The fields tried when a tool has no entry in [`BODY_FIELDS`]. */
const FALLBACK_FIELDS = ["text", "plan", "outline", "new"] as const

/** The call's parsed input as a plain object, when it is one. */
function inputRecord(call: ToolCallDto): Record<string, unknown> | undefined {
  const input = call.input
  if (input === null || input === undefined || typeof input !== "object" || Array.isArray(input)) {
    return undefined
  }
  return input as Record<string, unknown>
}

/** A non-empty string field of the call's input, or `""`. */
function stringField(record: Record<string, unknown> | undefined, key: string): string {
  const value = record?.[key]
  return typeof value === "string" ? value : ""
}

/** The text under review: the outline, the plan, or the prose a held write tool carries. */
export function heldBodyText(call: ToolCallDto): string {
  const record = inputRecord(call)
  const declared = stringField(record, BODY_FIELDS[call.name] ?? "")
  if (declared !== "") {
    return declared
  }
  for (const key of FALLBACK_FIELDS) {
    const value = stringField(record, key)
    if (value !== "") {
      return value
    }
  }
  return ""
}

/** The reference notes of a held `submit_outline`, or `""` (spec/05 §4). */
export function heldReferenceNotes(call: ToolCallDto): string {
  return call.name === "submit_outline" ? stringField(inputRecord(call), "reference_notes") : ""
}

/**
 * The call a `held` session is waiting on: `held_tool_call_id` resolved against the snapshot.
 *
 * The snapshot carries every tool call of its last 200 messages, so the held call is normally
 * there. A session held before that window (or a snapshot still loading) has no call to show; the
 * caller then keeps the generic controls instead of guessing.
 *
 * Only a `held` session resolves: `held_tool_call_id` is cleared when the call is finalized, so a
 * stale id on a non-held row must not resurrect a review block for a decision that was already
 * made.
 */
export function findHeldCall(
  session: SessionDto | undefined,
  calls: ToolCallDto[] | undefined,
  messages: MessageWithCallsDto[] | undefined,
): ToolCallDto | undefined {
  const id = session?.held_tool_call_id
  if (session?.status !== "held" || id === null || id === undefined) {
    return undefined
  }
  const fromCalls = calls?.find((call) => call.id === id)
  if (fromCalls !== undefined) {
    return fromCalls
  }
  for (const item of messages ?? []) {
    const match = item.tool_calls.find((call) => call.id === id)
    if (match !== undefined) {
      return match
    }
  }
  return undefined
}

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
  // The feedback box is a disclosure: closing it drops the draft, so a later open starts clean.
  const [changesOpen, setChangesOpen] = useState(false)
  const [feedback, setFeedback] = useState("")

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
  // The server treats blank feedback as no feedback and then *approves* the call
  // (`services/sessions.rs` `resume_inner` filters on `trim`). A whitespace-only "request changes"
  // must therefore be impossible from the UI.
  const trimmed = feedback.trim()

  const submit = () => {
    if (trimmed === "") {
      return
    }
    setChangesOpen(false)
    setFeedback("")
    onRequestChanges(trimmed)
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
          onClick={() => {
            setChangesOpen((open) => !open)
            setFeedback("")
          }}
        >
          <Pencil aria-hidden="true" />
          {t("agent:held.request_changes")}
        </Button>
      </div>

      {changesOpen ? (
        <div className="space-y-2">
          <Textarea
            value={feedback}
            autoFocus
            rows={3}
            aria-label={t("agent:held.feedback_label")}
            placeholder={t("agent:feedback.placeholder")}
            onChange={(event) => setFeedback(event.target.value)}
            onKeyDown={(event) => {
              // Ignore IME composition (AGENTS.md §6): Enter confirms Chinese and Japanese input.
              if (isComposing(event)) {
                return
              }
              if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
                event.preventDefault()
                submit()
              }
            }}
          />
          <p className="text-xs text-muted-foreground">{changesHint}</p>
          <Button size="sm" disabled={busy || trimmed === ""} onClick={submit}>
            {t("agent:feedback.submit")}
          </Button>
        </div>
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
