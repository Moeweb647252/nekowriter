import { useCallback, useEffect, useRef, useState } from "react"

import {
  clearWorkspaceDraft,
  readWorkspaceDraft,
  saveWorkspaceDraft,
  type DraftPhase,
  type WorkspaceDraft,
} from "@/lib/drafts"

type DraftIdentity = {
  dataDir?: string
  versionId: string
  phase: DraftPhase
  targetId: string | null
  baseRevision: string | null
}

type DraftPersistenceOptions = DraftIdentity & {
  text: string
  citations?: unknown[]
  dirty: boolean
  onRestore: (draft: WorkspaceDraft) => void
}

const emptyCitations: unknown[] = []

function identityKey(identity: DraftIdentity) {
  return JSON.stringify([
    identity.dataDir,
    identity.versionId,
    identity.phase,
    identity.targetId,
  ])
}

type PendingDraft = {
  dataDir?: string
  versionId: string
  phase: DraftPhase
  targetId: string | null
  baseRevision: string | null
  text: string
  citations: unknown[]
  dirty: boolean
}

type ClearedDraftValue = {
  identity: string
  text: string
  citations: unknown[]
}

/**
 * 在本地服务快照之外保存编辑中的文本。只有调用方判定为 dirty 后才写入，成功提交
 * 后必须显式 clear；草稿恢复绝不自动向服务端提交。
 */
export function useWorkspaceDraftPersistence({
  dataDir,
  versionId,
  phase,
  targetId,
  baseRevision,
  text,
  citations = emptyCitations,
  dirty,
  onRestore,
}: DraftPersistenceOptions) {
  const restoreRef = useRef(onRestore)
  const identity = identityKey({
    dataDir,
    versionId,
    phase,
    targetId,
    baseRevision,
  })
  // identity changes synchronously during render. Keeping the identity that has
  // actually been read prevents the previous target's text from being written
  // to the next chapter in the effect gap after a chapter switch.
  const [readyIdentity, setReadyIdentity] = useState(() =>
    dataDir === undefined ? identity : ""
  )
  const ready = readyIdentity === identity
  const pendingDraftsRef = useRef(new Map<string, PendingDraft>())
  const clearedValueRef = useRef<ClearedDraftValue | null>(null)
  const saveTimeoutRef = useRef<number | null>(null)
  const wasCleared =
    clearedValueRef.current?.identity === identity &&
    clearedValueRef.current.text === text &&
    clearedValueRef.current.citations === citations
  if (!wasCleared && clearedValueRef.current?.identity === identity) {
    clearedValueRef.current = null
  }
  const shouldPersist = dirty && !wasCleared

  pendingDraftsRef.current.set(identity, {
    dataDir,
    versionId,
    phase,
    targetId,
    baseRevision,
    text,
    citations,
    dirty: shouldPersist,
  })

  useEffect(() => {
    restoreRef.current = onRestore
  }, [onRestore])

  useEffect(() => {
    let active = true
    if (!dataDir) {
      setReadyIdentity(identity)
      return () => {
        active = false
      }
    }
    setReadyIdentity("")
    void readWorkspaceDraft(dataDir, versionId, phase, targetId)
      .then((draft) => {
        if (active && draft) restoreRef.current(draft)
      })
      .finally(() => {
        if (active) setReadyIdentity(identity)
      })
    return () => {
      active = false
    }
  }, [dataDir, identity, phase, targetId, versionId])

  // The debounce effect below is intentionally cancelled as text changes. On a
  // route or chapter change, preserve its latest value instead of letting that
  // cleanup discard a draft that has not reached the 400 ms delay yet.
  useEffect(() => {
    return () => {
      const draft = pendingDraftsRef.current.get(identity)
      if (!draft?.dataDir || !draft.dirty) return
      void saveWorkspaceDraft(draft.dataDir, {
        versionId: draft.versionId,
        phase: draft.phase,
        targetId: draft.targetId,
        baseRevision: draft.baseRevision,
        text: draft.text,
        citations: draft.citations,
      })
    }
  }, [identity])

  useEffect(() => {
    if (!dataDir || !ready || !shouldPersist) return
    const timeout = window.setTimeout(() => {
      if (saveTimeoutRef.current === timeout) {
        saveTimeoutRef.current = null
      }
      void saveWorkspaceDraft(dataDir, {
        versionId,
        phase,
        targetId,
        baseRevision,
        text,
        citations,
      })
    }, 400)
    saveTimeoutRef.current = timeout
    return () => {
      window.clearTimeout(timeout)
      if (saveTimeoutRef.current === timeout) {
        saveTimeoutRef.current = null
      }
    }
  }, [
    baseRevision,
    citations,
    dataDir,
    phase,
    ready,
    shouldPersist,
    targetId,
    text,
    versionId,
  ])

  const clear = useCallback(async () => {
    if (!dataDir) return
    if (saveTimeoutRef.current !== null) {
      window.clearTimeout(saveTimeoutRef.current)
      saveTimeoutRef.current = null
    }
    clearedValueRef.current = { identity, text, citations }
    const draft = pendingDraftsRef.current.get(identity)
    if (draft)
      pendingDraftsRef.current.set(identity, { ...draft, dirty: false })
    await clearWorkspaceDraft(dataDir, versionId, phase, targetId)
  }, [citations, dataDir, identity, phase, targetId, text, versionId])

  return { clear, ready }
}
