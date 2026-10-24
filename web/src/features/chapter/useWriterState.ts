/**
 * The chapter's writer session, as the reading column needs it (spec/11 §5.2).
 *
 * `ChapterDto.open_sessions` is authoritative (spec/10 §2.8). From it this hook derives the three
 * things `ChapterText` renders around the text:
 * - whether editing is locked (a **running** writer session, spec/03 §14);
 * - the held call's kind and target paragraphs, for the `HoldBar`;
 * - the live `insert_text` preview, for `GhostText`.
 */
import { useMemo } from "react"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { useSession } from "@/api/hooks/sessions"
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

/** The held call of a session, from its snapshot's tool calls. */
function heldCall(
  session: SessionDto | undefined,
  calls: ToolCallDto[] | undefined,
): ToolCallDto | undefined {
  if (session === undefined || session.held_tool_call_id === null) {
    return undefined
  }
  return calls?.find((call) => call.id === session.held_tool_call_id)
}

/** The paragraph pids a held text call touched, from its `ui` payload (spec/05 §4). */
export function heldPids(call: ToolCallDto | undefined): string[] {
  if (call === undefined || call.ui === null || typeof call.ui !== "object") {
    return []
  }
  const ui = call.ui as Record<string, unknown>
  const pids: string[] = []
  if (typeof ui.pid === "string") {
    pids.push(ui.pid)
  }
  if (Array.isArray(ui.inserted)) {
    for (const pid of ui.inserted) {
      if (typeof pid === "string") {
        pids.push(pid)
      }
    }
  }
  if (Array.isArray(ui.deleted)) {
    for (const pid of ui.deleted) {
      if (typeof pid === "string") {
        pids.push(pid)
      }
    }
  }
  return [...new Set(pids)]
}

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

  const held = useMemo(
    () => heldCall(session, snapshot.data?.tool_calls ?? calls),
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
    heldPids: heldPids(held),
    heldOutline: held?.name === "submit_outline",
    preview,
  }
}
