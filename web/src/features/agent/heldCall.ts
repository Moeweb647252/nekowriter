/**
 * The held-call decision as data, shared by every surface that offers it (PROJECT §8.6, spec/11
 * §5.2/§5.3, D-137, D-141).
 *
 * A `held` session waits on **one tool call** whose deferred effects the author must accept or
 * reject (spec/03 §5.2 `hold_condition`, §5.4). Two screens now offer that decision, and they must
 * agree on every part of it that is not a matter of wording:
 *
 * - **which call** is held ([`findHeldCall`]) — `SessionDto.held_tool_call_id` resolved against the
 *   snapshot, never guessed;
 * - **what it submitted** ([`heldBodyText`], [`heldReferenceNotes`]) and which wording it takes
 *   ([`heldReviewCase`]);
 * - **where it is decided** ([`isChapterWriteTool`]) — a held chapter write is decided in the draft
 *   pane's dock, because that is where its text would land (D-141); every other hold stays in the
 *   agent panel;
 * - **the two mutation payloads** ([`approveHeldInput`], [`requestChangesInput`]) — approve is
 *   `resume` with **no** feedback key (it finalizes the call with its effects); request changes is
 *   the same call with exactly the trimmed feedback (it revokes them and appends the revoke notice,
 *   spec/03 §5.4). A blank request-changes is not a payload at all: the server filters blank
 *   feedback to `None` and would therefore **approve** the call, a silent opposite outcome.
 *
 * This module is pure so both surfaces — and their tests — can share it without a React tree.
 */
import type { MessageWithCallsDto } from "@/api/gen/MessageWithCallsDto"
import type { PendingContent } from "@/api/gen/PendingContent"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"

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
 * The writer tools that change the chapter body (spec/03 §5.2 `WRITE_TOOLS`, plus `finish_draft`,
 * which closes the open tail and stages that change too).
 *
 * A held call of one of these tools is decided in the draft pane's dock (D-141), because the dock
 * can show the staged content merged into the draft it would replace; the agent panel then only
 * points at it, so the same call never has two differently-worded decision surfaces.
 */
const CHAPTER_WRITE_TOOLS = new Set([
  "insert_text",
  "insert_paragraphs",
  "replace_text",
  "rewrite_paragraph",
  "delete_paragraphs",
  "finish_draft",
])

/** True when the tool changes the chapter body, so its hold is decided in the draft pane's dock. */
export function isChapterWriteTool(name: string): boolean {
  return CHAPTER_WRITE_TOOLS.has(name)
}

/**
 * The staged chapter write a held call carries, or `undefined`.
 *
 * `pending_content` is `Some` only for a held write tool (D-139): `execute` computed the content
 * but did not store it, and the field is cleared once the call is finalized. It is the one signal
 * that the draft pane has text to show, so it also decides where the call is reviewed.
 */
export function stagedContent(call: ToolCallDto | undefined): PendingContent | undefined {
  return call?.pending_content ?? undefined
}

/** The surface that owns a held call's approve/request-changes decision. */
export type HeldDecisionSurface =
  /** The agent panel's `HeldCallReview`, next to the conversation. */
  | "panel"
  /**
   * The chapter pane's dock, under the merged text the write would produce (D-141).
   */
  | "dock"

/**
 * Which surface decides this held call — and therefore which one must **not** offer a second,
 * differently-worded pair of the same two mutations.
 *
 * A held chapter write belongs to the draft pane (D-141). Everything else stays in the panel, which
 * is where it has always been decided (D-137).
 */
export function heldDecisionSurface(call: ToolCallDto | undefined): HeldDecisionSurface {
  if (call === undefined || !isChapterWriteTool(call.name)) {
    return "panel"
  }
  // A write tool only has something to show in the pane when it staged content. A call from before
  // D-139 carries none, so the dock would be empty; the panel's status-driven controls keep that
  // decision rather than pointing at nothing.
  return stagedContent(call) === undefined ? "panel" : "dock"
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

/**
 * The paragraph pids a held text call touched, from its `ui` payload (spec/05 §4).
 *
 * `ui` carries data only (D-054): `pid` for a rewrite, `inserted` and `deleted` for the paragraph
 * tools. This is the one place the payload is read, so a surface that names the touched paragraphs
 * and a surface that renders them cannot disagree.
 */
export function heldTargetPids(call: ToolCallDto | undefined): string[] {
  if (call === undefined || call.ui === null || typeof call.ui !== "object") {
    return []
  }
  const ui = call.ui as Record<string, unknown>
  const pids: string[] = []
  if (typeof ui.pid === "string") {
    pids.push(ui.pid)
  }
  for (const key of ["inserted", "deleted"]) {
    const value = ui[key]
    if (!Array.isArray(value)) {
      continue
    }
    for (const pid of value) {
      if (typeof pid === "string") {
        pids.push(pid)
      }
    }
  }
  return [...new Set(pids)]
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
 * The variables `POST /sessions/:id/resume` is called with (`web/src/api/hooks/sessions.ts`).
 *
 * Spelled out here so both surfaces hand the mutation the same shape: the hook turns an absent (or
 * empty) `feedback` into `{}` — the approve path — and the server does the same with a `Some("")`.
 */
export type ResumeVariables = { sessionId: number; feedback?: string }

/**
 * The approve payload: `resume` with the session id and **nothing else**.
 *
 * There is deliberately no `feedback` key to send — not even an empty one — because
 * `resume_inner` filters blank feedback to `None` and would take the approve path anyway (the
 * opposite of what a caller meant).
 */
export function approveHeldInput(sessionId: number): ResumeVariables {
  return { sessionId }
}

/**
 * The request-changes payload: `resume` with exactly the trimmed feedback, or `undefined` while
 * there is none.
 *
 * `undefined` is not a cosmetic guard: the server reads blank feedback as "no feedback" and
 * finalizes the held call, so submitting a whitespace-only box would silently approve it.
 */
export function requestChangesInput(
  sessionId: number,
  feedback: string,
): ResumeVariables | undefined {
  const trimmed = feedback.trim()
  if (trimmed === "") {
    return undefined
  }
  return { sessionId, feedback: trimmed }
}
