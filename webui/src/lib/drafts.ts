import { useCallback, useState } from "react"

/** Where drafts are kept in this browser. */
const STORAGE_PREFIX = "nekowriter.draft."

interface StoredDraft {
  text: string
  /** The saved value the draft started from. */
  base: string
}

function read(key: string): unknown {
  try {
    const raw = localStorage.getItem(STORAGE_PREFIX + key)
    return raw === null ? null : JSON.parse(raw)
  } catch {
    return null
  }
}

function write(key: string, draft: unknown) {
  try {
    if (draft === null) localStorage.removeItem(STORAGE_PREFIX + key)
    else localStorage.setItem(STORAGE_PREFIX + key, JSON.stringify(draft))
  } catch {
    // Without storage the draft lives only as long as the page.
  }
}

export interface Draft {
  /** The draft if there is one, otherwise the saved value. */
  value: string
  setValue: (text: string) => void
  /** Whether the value differs from the saved one. */
  unsaved: boolean
  /** Whether the saved value changed after the draft was started (§18.5). */
  conflict: boolean
  /** Forgets the draft; call it once the server confirmed the value. */
  clear: () => void
}

/**
 * Unsent text, kept in this browser until the server confirms it (§18.5). `key` names the
 * input: the version, the phase and the target. IDs are unique across data directories,
 * so drafts of different data directories never mix.
 */
export function useDraft(key: string, saved: string): Draft {
  const [draft, setDraft] = useState(() => read(key) as StoredDraft | null)

  const setValue = useCallback(
    (text: string) => {
      setDraft((current) => {
        const next = text === saved ? null : { text, base: current?.base ?? saved }
        write(key, next)
        return next
      })
    },
    [key, saved],
  )

  const clear = useCallback(() => {
    write(key, null)
    setDraft(null)
  }, [key])

  return {
    value: draft?.text ?? saved,
    setValue,
    unsaved: draft !== null && draft.text !== saved,
    conflict: draft !== null && draft.base !== saved,
    clear,
  }
}

export interface DraftList<T> {
  items: T[]
  add: (item: T) => void
  remove: (index: number) => void
  /** Forgets the items; call it once the server confirmed them. */
  clear: () => void
}

/** Unsent items that go with a text draft, such as its citations, kept the same way. */
export function useDraftList<T>(key: string): DraftList<T> {
  const [items, setItems] = useState(() => (read(key) as T[] | null) ?? [])

  const update = useCallback(
    (change: (current: T[]) => T[]) => {
      setItems((current) => {
        const next = change(current)
        write(key, next.length === 0 ? null : next)
        return next
      })
    },
    [key],
  )

  const add = useCallback((item: T) => update((current) => [...current, item]), [update])
  const remove = useCallback(
    (index: number) => update((current) => current.filter((_, at) => at !== index)),
    [update],
  )
  const clear = useCallback(() => update(() => []), [update])
  return { items, add, remove, clear }
}

/** `text` added to the end of a draft, after a blank line unless the draft is empty. */
export function appended(draft: string, text: string): string {
  return draft.trim() === "" ? text : `${draft.trimEnd()}\n\n${text}`
}
