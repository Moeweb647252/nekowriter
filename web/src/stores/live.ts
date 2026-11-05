/** Streaming buffers and SSE connection state (spec/11 §4.3). */
import { create } from "zustand"

export type StreamKind = "text" | "reasoning"
export type ConnectionState = "connecting" | "open" | "reconnecting"

type Stream = {
  seq: number
  text: Record<number, string>
  reasoning: Record<number, string>
}

type Preview = { name: string; field: string; text: string; pinned?: boolean }

/** Writer tools whose preview is pinned across the persisted-message handoff (spec/11 §5.2). */
const PINNABLE_PREVIEW_TOOLS = new Set(["insert_text", "insert_paragraphs", "rewrite_paragraph"])

type LiveState = {
  connection: ConnectionState
  streams: Record<number, Stream>
  previews: Record<string, Preview>
  setConnection: (state: ConnectionState) => void
  applyDelta: (sessionId: number, seq: number, kind: StreamKind, index: number, text: string) => void
  applyPreview: (
    sessionId: number,
    seq: number,
    callIndex: number,
    name: string,
    field: string,
    delta: string,
  ) => void
  clearSeq: (sessionId: number, seq: number, keepWriterPreviews?: boolean) => void
  clearPreview: (sessionId: number, seq: number, callIndex: number) => void
  clearSessionPreviews: (sessionId: number) => void
}

export const useLiveStore = create<LiveState>()((set) => ({
  connection: "connecting",
  streams: {},
  previews: {},
  setConnection: (connection) => set({ connection }),
  applyDelta: (sessionId, seq, kind, index, text) =>
    set((state) => {
      const existing = state.streams[sessionId] ?? { seq, text: {}, reasoning: {} }
      if (existing.seq !== seq) {
        // A new message sequence supersedes the previous buffers.
        return {
          streams: {
            ...state.streams,
            [sessionId]: {
              seq,
              text: kind === "text" ? { [index]: text } : {},
              reasoning: kind === "reasoning" ? { [index]: text } : {},
            },
          },
        }
      }
      const bucket = kind === "text" ? existing.text : existing.reasoning
      return {
        streams: {
          ...state.streams,
          [sessionId]: {
            ...existing,
            [kind === "text" ? "text" : "reasoning"]: {
              ...bucket,
              [index]: (bucket[index] ?? "") + text,
            },
          },
        },
      }
    }),
  applyPreview: (sessionId, seq, callIndex, name, field, delta) =>
    set((state) => {
      const key = `${sessionId}:${seq}:${callIndex}`
      const existing = state.previews[key]
      return {
        previews: {
          ...state.previews,
          [key]: {
            name: previewName(existing, name),
            field: previewField(existing, field),
            text: (existing?.text ?? "") + delta,
          },
        },
      }
    }),
  clearSeq: (sessionId, seq, keepWriterPreviews = false) =>
    set((state) => {
      const streams = { ...state.streams }
      const current = streams[sessionId]
      if (current && current.seq === seq) {
        delete streams[sessionId]
      }
      const prefix = `${sessionId}:${seq}:`
      const previews = Object.fromEntries(
        Object.entries(state.previews).flatMap(([key, preview]) => {
          if (!key.startsWith(prefix)) return [[key, preview]]
          // The persisted message is not yet the visible draft for a writer tool: keep its
          // sentence pinned (no caret) until the call or chapter content replaces it. Other
          // previews are superseded immediately.
          if (keepWriterPreviews && PINNABLE_PREVIEW_TOOLS.has(preview.name)) {
            return [[key, { ...preview, pinned: true }]]
          }
          return []
        }),
      )
      return { streams, previews }
    }),
  clearPreview: (sessionId, seq, callIndex) =>
    set((state) => {
      const key = `${sessionId}:${seq}:${callIndex}`
      if (state.previews[key] === undefined) return state
      const previews = { ...state.previews }
      delete previews[key]
      return { previews }
    }),
  clearSessionPreviews: (sessionId) =>
    set((state) => ({
      previews: Object.fromEntries(
        Object.entries(state.previews).filter(([key]) => !key.startsWith(`${sessionId}:`)),
      ),
    })),
}))

function previewName(existing: Preview | undefined, name: string): string {
  return existing?.name ?? name ?? ""
}

function previewField(existing: Preview | undefined, field: string): string {
  return existing?.field ?? field ?? ""
}
