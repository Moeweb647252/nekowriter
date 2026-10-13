import { useTranslation } from "react-i18next"
import { Link } from "react-router"

import { ApiError } from "@/api/client"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useStartPreparation } from "@/api/queries/workspace"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button, buttonVariants } from "@/components/ui/button"
import type { Draft } from "@/lib/drafts"

/** Starting preparation once the brief is saved (§5.3). */
export function StartDock({ view, brief }: { view: WorkspaceView; brief: Draft }) {
  const { t } = useTranslation()
  const start = useStartPreparation(view.version.id)
  const modelNotReady =
    start.error instanceof ApiError && start.error.body.code === "MODEL_NOT_READY"

  return (
    <div className="flex flex-col items-start gap-3">
      <p>{t(($) => $.workspace.preparation.startHint)}</p>
      {(view.brief === null || brief.unsaved) && (
        <p className="text-sm text-muted-foreground">
          {t(($) => $.workspace.preparation.saveFirst)}
        </p>
      )}
      {start.isError && <ErrorAlert error={start.error} />}
      <div className="flex gap-2">
        <Button
          disabled={view.brief === null || brief.unsaved || start.isPending}
          onClick={() => start.mutate({ revision: view.revision })}
        >
          {t(($) => $.workspace.preparation.start)}
        </Button>
        {modelNotReady && (
          <Link to="/settings" className={buttonVariants({ variant: "outline" })}>
            {t(($) => $.common.nav.settings)}
          </Link>
        )}
      </div>
    </div>
  )
}
