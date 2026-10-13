import { useMemo, useRef } from "react"

import type { CommandId } from "./generated/CommandId"

export function newCommandId(): CommandId {
  return crypto.randomUUID()
}

/**
 * Command IDs for one place in the interface that submits commands.
 *
 * Submitting the same input again (a retry after a failure) reuses the previous ID, so
 * the server applies the action at most once even if the first response was lost. A
 * different input, or any input after a success, gets a new ID.
 */
export function useCommandIds() {
  const pending = useRef<{ key: string; id: CommandId } | null>(null)

  return useMemo(
    () => ({
      idFor(input: unknown): CommandId {
        const key = JSON.stringify(input)
        if (pending.current?.key !== key) pending.current = { key, id: newCommandId() }
        return pending.current.id
      },
      succeeded() {
        pending.current = null
      },
    }),
    [],
  )
}
