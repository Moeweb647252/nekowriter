/**
 * The chapter's writer session, as the draft pane needs it (spec/11 §5.2, D-141).
 *
 * `ChapterDto.open_sessions` is authoritative (spec/10 §2.8). From it this hook derives what the
 * pane renders around the text:
 * - whether editing is locked (a **running** writer session, spec/03 §14);
 * - the held call, resolved through the shared `findHeldCall` so the pane and the agent panel agree
 *   on which call is being decided;
 * - its target paragraphs and whether it is the outline hold, for the inline `HoldBar`;
 * - the staged chapter write (`pending_content`) and how it stands against the draft, for the dock;
 * - the live `insert_text` preview, for `GhostText`.
 */
import { useMemo } from "react"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { useSession } from "@/api/hooks/sessions"
import { findHeldCall, heldTargetPids, isChapterWriteTool } from "@/features/agent/heldCall"
import { pendingWrite, type PendingWrite } from "@/features/chapter/pendingWrite"
import { useLiveStore } from "@/stores/live"
import type { Preview } from "@/features/chapter/GhostText"

/** A writer session of any phase (write or revise). */
function isWriter(session: SessionDto): boolean {
  return session.kind === "chapter_write" || session.kind === "chapter_revise"
}

/** The chapter's open writer session, if there is one (spec/10 §2.8). */
export function openWriterSession(chapter: ChapterDto | undefined): SessionDto | undefined {
  return chapter?.open_sessions?.find(isWriter)
}

/**
 * The paragraph pids a held call touched (`heldTargetPids` in the shared module, re-exported here
 * because the reading column's callers and tests import it from this hook).
 */
export const heldPids = heldTargetPids

export type WriterState = {
  /** The chapter's open writer session. */
  session: SessionDto | undefined
  /** True while the writer is running: author edits are rejected (spec/03 §14). */
  locked: boolean
  /** The held call, when the session is held. */
  held: ToolCallDto | undefined
  /** The held call's target paragraphs. */
  heldPids: string[]
  /** True when the held call is `submit_outline` (the outline variant of the hold bar). */
  heldOutline: boolean
  /**
   * True when the held call is a chapter write, so its decision belongs to the draft pane's dock
   * rather than to the inline `HoldBar` (D-141). The pane then renders the dock, and neither the
   * `HoldBar` nor the agent panel offers a second decision for the same call.
   */
  heldWrite: boolean
  /** The staged chapter write of the held call, when it carries one (D-139). */
  pendingWrite: PendingWrite | undefined
  /** The live preview of the running writer's `insert_text`, if any. */
  preview: Preview | undefined
}

/** Derives the writer state of an open chapter. */
export function useWriterState(
  chapter: ChapterDto | undefined,
  calls: ToolCallDto[] | undefined,
): WriterState {
  const session = useMemo(() => openWriterSession(chapter), [chapter])
  const previews = useLiveStore((state) => state.previews)
  const snapshot = useSession(session?.status === "held" ? (session?.id ?? null) : null)

  // One resolver for both surfaces (D-141): the pane must never decide a different call than the
  // panel names.
  const held = useMemo(
    () =>
      findHeldCall(session, snapshot.data?.tool_calls ?? calls, snapshot.data?.messages),
    [session, snapshot.data, calls],
  )

  const preview = useMemo(() => {
    if (session === undefined || session.status !== "running") {
      return undefined
    }
    // The preview key is `sessionId:seq:callIndex`; the newest seq of this session is the live one
    // (spec/03 §12). A persisted call supersedes its preview, so a stale key cannot linger.
    const prefix = `${session.id}:`
    let newest: { seq: number; key: string } | undefined
    for (const key of Object.keys(previews)) {
      if (!key.startsWith(prefix)) {
        continue
      }
      const seq = Number(key.slice(prefix.length).split(":")[0])
      if (!Number.isFinite(seq)) {
        continue
      }
      if (newest === undefined || seq > newest.seq) {
        newest = { seq, key }
      }
    }
    return newest === undefined ? undefined : previews[newest.key]
  }, [previews, session])

  return {
    session,
    locked: session?.status === "running",
    held,
    heldPids: heldTargetPids(held),
    heldOutline: held?.name === "submit_outline",
    heldWrite: held !== undefined && isChapterWriteTool(held.name),
    pendingWrite: pendingWrite(held),
    preview,
  }
}
