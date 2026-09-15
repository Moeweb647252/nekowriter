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

/** 一份尚未落盘的编辑内容，供 identity 变化或面板卸载时补写。 */
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

/** 已确认提交成功的内容：同一份内容不再落盘，直到用户重新编辑。 */
type ClearedDraft = {
  identity: string
  text: string
  citations: unknown[]
}

/**
 * 在本地服务快照之外保存编辑中的文本。只有调用方判定为 dirty 后才写入，成功提交
 * 后必须显式 clear；草稿恢复绝不自动向服务端提交。
 *
 * render 只读取 state：最近一次编辑内容镜像在 effect 里维护，identity 变化时的补写
 * 读那份镜像，因此被重复执行或被丢弃的一次 render 不会改变落盘行为。
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
  const identity = identityKey({
    dataDir,
    versionId,
    phase,
    targetId,
    baseRevision,
  })
  const restoreRef = useRef(onRestore)
  /** 已经为哪个 identity 读完过草稿；读取完成前不写入，避免覆盖尚未恢复的草稿。 */
  const [readyIdentity, setReadyIdentity] = useState("")
  const [clearedDraft, setClearedDraft] = useState<ClearedDraft | null>(null)
  const pendingRef = useRef<PendingDraft | null>(null)
  const saveTimeoutRef = useRef<number | null>(null)

  // 没有数据目录就没有草稿库，此时不需要等待任何读取。
  const ready = dataDir === undefined || readyIdentity === identity
  const alreadyCleared =
    clearedDraft?.identity === identity &&
    clearedDraft.text === text &&
    clearedDraft.citations === citations
  const shouldPersist = dirty && !alreadyCleared

  useEffect(() => {
    restoreRef.current = onRestore
  }, [onRestore])

  // 只有这个 effect 会把 render 的可见值搬进 ref，补写路径因此读到的是最新内容。
  useEffect(() => {
    pendingRef.current = {
      dataDir,
      versionId,
      phase,
      targetId,
      baseRevision,
      text,
      citations,
      dirty: shouldPersist,
    }
  })

  useEffect(() => {
    if (!dataDir) return
    let active = true
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

  // 草稿的防抖写入会随 text 变化重排，cleanup 因此会取消定时器。切章或卸载时改用
  // 补写：否则一次 400ms 内完成的切换会把刚打的一段字丢掉。
  useEffect(() => {
    return () => {
      const draft = pendingRef.current
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
    setClearedDraft({ identity, text, citations })
    if (pendingRef.current) {
      pendingRef.current = { ...pendingRef.current, dirty: false }
    }
    await clearWorkspaceDraft(dataDir, versionId, phase, targetId)
  }, [citations, dataDir, identity, phase, targetId, text, versionId])

  return { clear, ready }
}
