/** The open chapter document (spec/11 §4.3). */
import { create } from "zustand"

import type { ChapterContent } from "@/api/gen/ChapterContent"
import type { ContentOp } from "@/api/gen/ContentOp"

export type HighlightKind = "inserted" | "updated"
export type Highlight = { kind: HighlightKind; at: number }
export type ApplyResult = "applied" | "gap"

type ChapterDocState = {
  chapterId: number | null
  version: number
  paragraphs: { pid: string; text: string }[]
  tailOpen: boolean
  highlights: Record<string, Highlight>
  load: (chapterId: number, version: number, content: ChapterContent) => void
  applyOps: (version: number, ops: ContentOp[], actor: "agent" | "user") => ApplyResult
  clearHighlight: (pid: string) => void
  reset: () => void
}

export const useChapterDoc = create<ChapterDocState>()((set, get) => ({
  chapterId: null,
  version: 0,
  paragraphs: [],
  tailOpen: false,
  highlights: {},
  load: (chapterId, version, content) =>
    set({
      chapterId,
      version,
      paragraphs: content.paragraphs.map((p) => ({ ...p })),
      tailOpen: content.tail_open,
      highlights: {},
    }),
  applyOps: (version, ops, actor) => {
    const state = get()
    // Ignore a duplicate event (our own mutation response arrived first, or a replay).
    if (version <= state.version) {
      return "applied"
    }
    if (version !== state.version + 1) {
      return "gap"
    }
    let paragraphs = state.paragraphs.map((p) => ({ ...p }))
    let tailOpen = state.tailOpen
    const highlights = { ...state.highlights }
    for (const op of ops) {
      switch (op.op) {
        case "insert": {
          // An anchor the local document does not have means the two have diverged; inserting at
          // index 0 would silently corrupt the order, so report a gap and let the caller refetch
          // (spec/10 §3: "apply the ops if version = local + 1, else refetch").
          const anchor = paragraphs.findIndex((p) => p.pid === op.after)
          if (op.after !== null && anchor < 0) {
            return "gap"
          }
          paragraphs.splice(anchor + 1, 0, { pid: op.pid, text: op.text })
          if (actor === "agent") {
            highlights[op.pid] = { kind: "inserted", at: Date.now() }
          }
          break
        }
        case "update": {
          const index = paragraphs.findIndex((p) => p.pid === op.pid)
          if (index < 0) {
            return "gap"
          }
          paragraphs[index] = { ...paragraphs[index], text: op.text }
          if (actor === "agent") {
            highlights[op.pid] = { kind: "updated", at: Date.now() }
          }
          break
        }
        case "delete": {
          if (!paragraphs.some((p) => p.pid === op.pid)) {
            return "gap"
          }
          paragraphs = paragraphs.filter((p) => p.pid !== op.pid)
          delete highlights[op.pid]
          break
        }
        case "tail": {
          tailOpen = op.open
          break
        }
      }
    }
    set({ version, paragraphs, tailOpen, highlights })
    return "applied"
  },
  clearHighlight: (pid) =>
    set((state) => {
      const highlights = { ...state.highlights }
      delete highlights[pid]
      return { highlights }
    }),
  reset: () =>
    set({ chapterId: null, version: 0, paragraphs: [], tailOpen: false, highlights: {} }),
}))
