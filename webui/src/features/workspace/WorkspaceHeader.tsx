import { ArrowLeftIcon, PanelLeftIcon } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Link } from "react-router"

import { ProjectSettingsDialog } from "./ProjectSettingsDialog"
import { useStageLabel } from "./stage"
import { VersionMenu } from "./VersionMenu"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { Badge } from "@/components/ui/badge"
import { Button, buttonVariants } from "@/components/ui/button"

export function WorkspaceHeader({
  view,
  onShowContents,
}: {
  view: WorkspaceView
  onShowContents: () => void
}) {
  const { t } = useTranslation()
  const stageLabel = useStageLabel()

  return (
    <header className="flex h-14 shrink-0 items-center gap-3 border-b px-4">
      <Button
        variant="ghost"
        size="icon-sm"
        className="min-[1100px]:hidden"
        aria-label={t(($) => $.workspace.nav.open)}
        onClick={onShowContents}
      >
        <PanelLeftIcon />
      </Button>
      <Link to="/works" className={buttonVariants({ variant: "ghost", size: "sm" })}>
        <ArrowLeftIcon />
        {t(($) => $.workspace.backToWorks)}
      </Link>
      <div className="min-w-0">
        <h1 className="truncate font-heading text-sm font-semibold">{view.version.novelTitle}</h1>
        <p className="truncate text-xs text-muted-foreground">{view.project.name}</p>
      </div>
      <VersionMenu view={view} />
      <Badge variant="secondary" className="hidden sm:inline-flex">
        {stageLabel(view.version.stage)}
      </Badge>
      <div className="ml-auto">
        <ProjectSettingsDialog project={view.project} />
      </div>
    </header>
  )
}
