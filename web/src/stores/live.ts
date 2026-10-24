/** Streaming buffers and SSE connection state (spec/11 §4.3). */
import { create } from "zustand"

export type StreamKind = "text" | "reasoning"
export type ConnectionState = "connecting" | "open" | "reconnecting"

type Stream = {
  seq: number
  text: Record<number, string>
  reasoning: Record<number, string>
}

type Preview = { name: string; field: string; text: string }

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
  clearSeq: (sessionId: number, seq: number) => void
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
  clearSeq: (sessionId, seq) =>
    set((state) => {
      const streams = { ...state.streams }
      const current = streams[sessionId]
      if (current && current.seq === seq) {
        delete streams[sessionId]
      }
      const previews = Object.fromEntries(
        Object.entries(state.previews).filter(([key]) => !key.startsWith(`${sessionId}:${seq}:`)),
      )
      return { streams, previews }
    }),
}))

function previewName(existing: Preview | undefined, name: string): string {
  return existing?.name ?? name ?? ""
}

function previewField(existing: Preview | undefined, field: string): string {
  return existing?.field ?? field ?? ""
}
