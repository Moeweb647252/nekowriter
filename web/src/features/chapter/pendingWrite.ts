/**
 * The draft pane's view of a held chapter write (PROJECT §8.5, spec/03 §5.2, D-139, D-141).
 *
 * A held write tool never touches `chapters.content` (D-139): `execute` computes the content it
 * would produce and stores it on the call as `ToolCallDto.pending_content`, leaving the draft
 * alone. `pending_content.content` is a **complete** `ChapterContent` snapshot — the document it
 * would leave behind, not a delta on the confirmed one (`crates/server/src/agent/tools/writer.rs`
 * `stage_content` clones the whole result; `runner.rs::apply_pending_content` stores `write.content`
 * verbatim and derives the events from the difference). The pane can therefore show the author the
 * **final shape** the draft would take, which is exactly what they are being asked to accept.
 *
 * This module derives that view and says honestly whether it will apply:
 *
 * - **pending and current** (`expected_version` still equals the draft's `content_version`):
 *   approving applies the staged content exactly, so the merged document is what the chapter
 *   becomes. Paragraphs the write adds or changes are marked as waiting for the decision.
 * - **pending but superseded** (`expected_version` no longer matches): `apply_pending_content`
 *   drops the write and keeps the author's edit. Rendering the staged text would show a chapter
 *   that will never exist, so the pane keeps the confirmed draft and the dock explains what
 *   happened instead.
 * - **no staged write** (a hold that is not a write, or a call that predates D-139): the pane shows
 *   the confirmed draft exactly as before.
 *
 * Nothing here reads the network or a store: callers pass the call they resolved and the draft on
 * screen, so the rules are testable on their own.
 */
import type { ChapterContent } from "@/api/gen/ChapterContent"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { stagedContent } from "@/features/agent/heldCall"
import { countWords } from "@/lib/wordcount"

/** Which part of the merged document a paragraph belongs to. */
export type PendingParagraphKind =
  /** Confirmed text the staged write leaves unchanged. */
  | "confirmed"
  /** Confirmed text the staged write rewrites. */
  | "changed"
  /** Text the staged write adds; it does not exist in the confirmed draft. */
  | "added"

/** One paragraph of the merged document. */
export type PendingParagraph = {
  pid: string
  text: string
  kind: PendingParagraphKind
  /** Character offset where the staged text starts diverging from the confirmed one (D-141). */
  highlightFrom?: number
}

/** The draft the pane is showing: the store's paragraphs and their version. */
export type DraftSnapshot = {
  version: number
  paragraphs: { pid: string; text: string }[]
}

/** The staged write of a held call, as the pane needs it. */
export type PendingWrite = {
  /** The complete document the write would store (not a delta — see the module comment). */
  content: ChapterContent
  /** The `chapters.content_version` the tool read (spec/03 §5.2). */
  expectedVersion: number
}

/**
 * How a staged write stands against the draft on screen.
 *
 * - `current`: it was computed from this very version, so approving applies it exactly.
 * - `superseded`: the draft has moved on (the author edited it, or a later write landed), so
 *   `apply_pending_content` discards it — the author's text wins and the staged text is never
 *   applied.
 */
export type PendingStanding = "current" | "superseded"

/**
 * The staged write a held call carries, or `undefined` when there is none.
 *
 * The call's own status must be `held`: `pending_content` is `None` once the call is finalized
 * (spec/03 §5.2), but a snapshot fetched just before the decision could still carry one, and a
 * decided call must never keep a preview on screen.
 */
export function pendingWrite(call: ToolCallDto | undefined): PendingWrite | undefined {
  // The staged-content accessor lives with the rest of the held-call logic, so this module and the
  // decision-surface rule cannot disagree about whether a call has anything staged.
  const pending = stagedContent(call)
  if (call === undefined || call.status !== "held" || pending === undefined) {
    return undefined
  }
  return { content: pending.content, expectedVersion: pending.expected_version }
}

/**
 * Whether a staged write would still apply to this draft.
 *
 * Mirrors `runner::apply_pending_content`, which discards a write whose `expected_version` is not
 * the chapter's current `content_version` (`stale = chapter.content_version !=
 * write.expected_version`). The pane must not show a merged chapter for a write that will be
 * dropped, so this is the gate for the merged view. The runner's one exception — a stale write is
 * still applied when the chapter already equals it, an idempotency escape for a retried
 * finalization — produces the same document either way and needs no separate case.
 */
export function pendingStanding(
  write: PendingWrite | undefined,
  draftVersion: number,
): PendingStanding | undefined {
  if (write === undefined) {
    return undefined
  }
  return write.expectedVersion === draftVersion ? "current" : "superseded"
}

/**
 * The merged document: the staged content in its own order and text, with each paragraph marked by
 * whether it is untouched confirmed text, confirmed text the write changes, or text the write adds.
 *
 * The staged content decides the *shape* — order, pids, text — because applying it replaces the
 * document. The confirmed draft is consulted only to classify each paragraph, which is what the
 * highlight means: "this is what changes if you approve".
 */
export function mergedPending(
  write: PendingWrite,
  draft: DraftSnapshot,
): PendingParagraph[] {
  const before = new Map<string, string>()
  for (const paragraph of draft.paragraphs) {
    before.set(paragraph.pid, paragraph.text)
  }
  return write.content.paragraphs.map((paragraph) => {
    const previous = before.get(paragraph.pid)
    if (previous === undefined) {
      return { pid: paragraph.pid, text: paragraph.text, kind: "added" }
    }
    if (previous !== paragraph.text) {
      // A rewrite that only appends to an existing paragraph must tint the appended part, not the
      // whole paragraph: the author asked for the highlight to start where the text diverges.
      let shared = 0
      const limit = Math.min(previous.length, paragraph.text.length)
      while (shared < limit && previous[shared] === paragraph.text[shared]) shared += 1
      return { pid: paragraph.pid, text: paragraph.text, kind: "changed", highlightFrom: shared }
    }
    return { pid: paragraph.pid, text: paragraph.text, kind: "confirmed" }
  })
}

/**
 * The word count of a `ChapterContent`, matching the server's `text::content_word_count`
 * (`crates/server/src/text/content.rs`), which folds the shared UAX #29 count over the paragraph
 * texts. Used for the staged projection the body reports next to the header's confirmed count.
 */
export function contentWordCount(content: ChapterContent): number {
  return content.paragraphs.reduce((total, paragraph) => total + countWords(paragraph.text), 0)
}

/** Everything the pane needs to decide how to render a held write. */
export type PendingDraftView = {
  standing: PendingStanding
  /** The version the write read, for the dock's explanation when it is superseded. */
  expectedVersion: number
  /** The draft's version now. */
  draftVersion: number
  /** The merged document; only meaningful while the write is `current`. */
  paragraphs: PendingParagraph[]
  /** Words in the merged document (what the chapter would read after approval). */
  stagedWords: number
  /** Words in the confirmed draft (what the header counts, spec/11 §8). */
  confirmedWords: number
}

/**
 * The pane's view of a held write against the draft on screen, or `undefined` when the call carries
 * no staged write (the pane then renders the confirmed draft unchanged).
 *
 * `draft.paragraphs` is the document the pane is rendering, not `chapter.content` from the last
 * query response: those differ while the author is typing, and the comparison above has to be
 * against what the author is looking at.
 */
export function pendingDraftView(
  call: ToolCallDto | undefined,
  draft: DraftSnapshot,
): PendingDraftView | undefined {
  const write = pendingWrite(call)
  const standing = pendingStanding(write, draft.version)
  if (write === undefined || standing === undefined) {
    return undefined
  }
  return {
    standing,
    expectedVersion: write.expectedVersion,
    draftVersion: draft.version,
    paragraphs: mergedPending(write, draft),
    stagedWords: contentWordCount(write.content),
    confirmedWords: draft.paragraphs.reduce(
      (total, paragraph) => total + countWords(paragraph.text),
      0,
    ),
  }
}
