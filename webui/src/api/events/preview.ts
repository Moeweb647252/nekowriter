import { createContext, useContext, useSyncExternalStore } from "react"

import type { ModelRequestId } from "../generated/ModelRequestId"
import type { Preview } from "../generated/Preview"
import type { RunId } from "../generated/RunId"

interface PreviewState {
  /** The preview of the request in flight. */
  current: Preview | null
  /** The most recent preview, kept after its request ended. */
  last: Preview | null
}

/**
 * The latest streaming preview from the event stream. A preview is not content: it is
 * replaced as the answer grows and cleared when the answer ends.
 */
export class PreviewStore {
  private state: PreviewState = { current: null, last: null }
  private readonly listeners = new Set<() => void>()

  readonly get = (): PreviewState => this.state

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  set(preview: Preview | null) {
    this.state = { current: preview, last: preview ?? this.state.last }
    for (const listener of this.listeners) listener()
  }
}

export const PreviewContext = createContext(new PreviewStore())

/**
 * The preview of the run's request while it streams. Once the answer ends, its last
 * preview stays until the workspace no longer shows that request in flight, so the text
 * does not blink out before the committed result replaces it.
 */
export function usePreview(
  runId: RunId | undefined,
  requestInFlight: ModelRequestId | null | undefined,
): Preview | null {
  const store = useContext(PreviewContext)
  const { current, last } = useSyncExternalStore(store.subscribe, store.get)
  if (current !== null && current.runId === runId) return current
  if (last !== null && last.runId === runId && last.requestId === requestInFlight) return last
  return null
}
