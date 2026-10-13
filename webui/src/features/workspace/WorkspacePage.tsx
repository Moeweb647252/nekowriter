import { useTranslation } from "react-i18next"
import { Link, Navigate, useParams } from "react-router"

import { PreparationWorkspace } from "./preparation/PreparationWorkspace"
import { workspacePath } from "./stage"
import { WritingWorkspace } from "./writing/WritingWorkspace"
import type { VersionId } from "@/api/generated/VersionId"
import { useWorkspace } from "@/api/queries/workspace"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { buttonVariants } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { assertNever } from "@/lib/assert"

/** A version's workspace, at `/works/:projectId/versions/:versionId`. */
export function WorkspacePage() {
  const { t } = useTranslation()
  const params = useParams<{ projectId: string; versionId: VersionId }>()
  const versionId = params.versionId ?? ""
  const workspace = useWorkspace(versionId)

  // Once loaded, the workspace stays on screen even if a later refresh fails.
  const view = workspace.data
  if (view === undefined) {
    return workspace.isError ? (
      <div className="mx-auto flex max-w-xl flex-col gap-4 px-4 py-16">
        <ErrorAlert error={workspace.error} onRetry={() => void workspace.refetch()} />
        <Link to="/works" className={buttonVariants({ variant: "outline" })}>
          {t(($) => $.workspace.backToWorks)}
        </Link>
      </div>
    ) : (
      <div role="status" className="flex h-svh items-center justify-center">
        <Spinner className="size-6" />
        <span className="sr-only">{t(($) => $.common.status.loading)}</span>
      </div>
    )
  }

  if (view.project.id !== params.projectId) {
    return <Navigate to={workspacePath(view.project.id, view.version.id)} replace />
  }
  const stage = view.version.stage
  switch (stage.phase) {
    case "preparation":
      return <PreparationWorkspace view={view} step={stage.step} />
    case "chapter":
    case "completed":
      return <WritingWorkspace view={view} />
    default:
      return assertNever(stage)
  }
}
