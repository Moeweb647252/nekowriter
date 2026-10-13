import { createContext, useContext, useSyncExternalStore } from "react"

import type { Preview } from "../generated/Preview"
import type { RunId } from "../generated/RunId"

/**
 * The latest streaming preview from the event stream. A preview is not content: it is
 * replaced as the answer grows and cleared when the answer ends.
 */
export class PreviewStore {
  private current: Preview | null = null
  private readonly listeners = new Set<() => void>()

  readonly get = (): Preview | null => this.current

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  set(preview: Preview | null) {
    this.current = preview
    for (const listener of this.listeners) listener()
  }
}

export const PreviewContext = createContext(new PreviewStore())

/** The preview of the run's request in flight, while one is streaming. */
export function usePreview(runId: RunId | undefined): Preview | null {
  const store = useContext(PreviewContext)
  const preview = useSyncExternalStore(store.subscribe, store.get)
  return preview !== null && preview.runId === runId ? preview : null
}
