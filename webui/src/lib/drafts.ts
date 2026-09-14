import { createStore, del, get, set } from "idb-keyval"

/** 仅保存尚未提交到本地服务的编辑草稿；模型凭据与服务设置永不进入浏览器草稿库。 */
export type DraftPhase =
  | "brief"
  | "preparation"
  | "chapter_guidance"
  | "chapter_outline"
  | "chapter_body"
  | "feedback"

export type WorkspaceDraft = {
  versionId: string
  phase: DraftPhase
  targetId: string | null
  baseRevision: string | null
  text: string
  citations: unknown[]
  updatedAtMs: number
}

const draftStore = createStore("nekowriter-workspace", "drafts")

function keyFor(
  dataDir: string,
  versionId: string,
  phase: DraftPhase,
  targetId: string | null
) {
  return JSON.stringify([dataDir, versionId, phase, targetId])
}

export async function readWorkspaceDraft(
  dataDir: string,
  versionId: string,
  phase: DraftPhase,
  targetId: string | null
) {
  return get<WorkspaceDraft>(
    keyFor(dataDir, versionId, phase, targetId),
    draftStore
  )
}

export async function saveWorkspaceDraft(
  dataDir: string,
  draft: Omit<WorkspaceDraft, "updatedAtMs">
) {
  const value: WorkspaceDraft = { ...draft, updatedAtMs: Date.now() }
  await set(
    keyFor(dataDir, value.versionId, value.phase, value.targetId),
    value,
    draftStore
  )
  return value
}

export async function clearWorkspaceDraft(
  dataDir: string,
  versionId: string,
  phase: DraftPhase,
  targetId: string | null
) {
  await del(keyFor(dataDir, versionId, phase, targetId), draftStore)
}
