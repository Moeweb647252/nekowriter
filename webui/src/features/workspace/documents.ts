import { useCallback } from "react"
import { useSearchParams } from "react-router"

import type { ArtifactKind } from "@/api/generated/ArtifactKind"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"

/** A preparation document the workspace can show: the brief or one of the artifacts. */
export type DocumentKey = "brief" | ArtifactKind

const DOCUMENTS: readonly string[] = [
  "brief",
  "background",
  "research",
  "characters",
  "outline",
] satisfies readonly DocumentKey[]

function isDocumentKey(value: string | null): value is DocumentKey {
  return value !== null && DOCUMENTS.includes(value)
}

/**
 * The document the workspace shows, kept in the address (`?doc=`) so it survives reloads.
 * Without a choice, `fallback` is shown; `null` means the stage's own view.
 */
export function useSelectedDocument(fallback: DocumentKey | null) {
  const [params, setParams] = useSearchParams()
  const chosen = params.get("doc")
  const select = useCallback(
    (doc: DocumentKey | null) => {
      setParams(doc === null ? {} : { doc }, { replace: true })
    },
    [setParams],
  )
  return { selected: isDocumentKey(chosen) ? chosen : fallback, select }
}

/** The first artifact waiting for a decision, to review first. */
export function firstOpenReview(view: WorkspaceView): ArtifactKind | null {
  return view.artifacts.find((artifact) => artifact.review?.status === "open")?.kind ?? null
}
