import { DownloadIcon } from "lucide-react"
import { useTranslation } from "react-i18next"

import { workspaceApi } from "@/api/endpoints/workspace"
import type { ExportFormat } from "@/api/generated/ExportFormat"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"

const FORMATS: readonly ExportFormat[] = ["markdown", "text"]

/** Downloads the version's committed chapters as Markdown or plain text (§16.4). */
export function ExportMenu({ view }: { view: WorkspaceView }) {
  const { t } = useTranslation()
  return (
    <DropdownMenu>
      <DropdownMenuTrigger render={<Button variant="outline" size="sm" />}>
        <DownloadIcon />
        {t(($) => $.workspace.export.label)}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {FORMATS.map((format) => (
          <DropdownMenuItem
            key={format}
            render={<a href={workspaceApi.exportUrl(view.version.id, format)} download />}
          >
            {t(($) => $.workspace.export.format[format])}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
