/**
 * The derived state of a chapter's outline (PROJECT §8.5, spec/11 §5.2, D-146).
 *
 * The server stores only `chapters.outline`; the outline's step state is derived here so the
 * header action table and the Outline tab can never disagree:
 * - `missing`: the outline is empty and no outline session is open;
 * - `in_progress`: an outline session is open (running, paused, stopped, failed, …);
 * - `awaiting_approval`: that session holds `submit_outline`;
 * - `accepted`: the outline is non-empty and no outline session is open.
 */
import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"

export type OutlineState = "missing" | "in_progress" | "awaiting_approval" | "accepted"

/** The chapter's open outline session, if there is one (spec/10 §2.8 `open_sessions`). */
export function openOutlineSession(chapter: ChapterDto | undefined): SessionDto | undefined {
  return chapter?.open_sessions?.find((session) => session.kind === "chapter_outline")
}

/** The outline state of a chapter. `calls` refines a held session into `awaiting_approval`. */
export function outlineState(
  chapter: ChapterDto | undefined,
  calls?: readonly ToolCallDto[],
): OutlineState {
  if (chapter === undefined) {
    return "missing"
  }
  const session = openOutlineSession(chapter)
  if (session !== undefined) {
    if (session.status === "held") {
      const held = calls?.find((call) => call.id === session.held_tool_call_id)
      // While the snapshot loads (`calls === undefined`) the only held outline call we can name
      // is the approval. Once the window is known and the held id is outside it, there is no
      // proposal to show: keep the session in progress instead of an empty approval UI (R3).
      if (calls === undefined) {
        return "awaiting_approval"
      }
      return held?.name === "submit_outline" ? "awaiting_approval" : "in_progress"
    }
    return "in_progress"
  }
  return chapter.outline.trim() === "" ? "missing" : "accepted"
}
