import { useCallback } from "react"
import { useTranslation } from "react-i18next"

import type { WorkflowStage } from "@/api/generated/WorkflowStage"
import type { ProjectId } from "@/api/generated/ProjectId"
import type { VersionId } from "@/api/generated/VersionId"
import { assertNever } from "@/lib/assert"

export function workspacePath(projectId: ProjectId, versionId: VersionId): string {
  return `/works/${projectId}/versions/${versionId}`
}

/** Names where a version is in the story. */
export function useStageLabel(): (stage: WorkflowStage) => string {
  const { t } = useTranslation()
  return useCallback(
    (stage: WorkflowStage) => {
      switch (stage.phase) {
        case "preparation":
          return t(($) => $.workspace.stage.preparation[stage.step])
        case "chapter":
          return t(($) => $.workspace.stage.chapter[stage.step])
        case "completed":
          return t(($) => $.workspace.stage.completed)
        default:
          return assertNever(stage)
      }
    },
    [t],
  )
}
