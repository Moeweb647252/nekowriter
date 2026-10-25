/** The global research page (PROJECT §8.10): every research task, with a create dialog. */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { NewResearchTaskDialog } from "@/features/research/NewResearchTaskDialog"
import { ResearchTaskList } from "@/features/research/ResearchTaskList"

export default function ResearchPage() {
  const { t } = useTranslation()
  const [creating, setCreating] = useState(false)

  return (
    <div className="mx-auto max-w-3xl space-y-4 p-6">
      <div>
        <h1 className="text-base font-semibold">{t("knowledge:research.title")}</h1>
        <p className="text-sm text-muted-foreground">{t("knowledge:research.list_hint")}</p>
      </div>
      <ResearchTaskList onCreate={() => setCreating(true)} />
      <NewResearchTaskDialog open={creating} onOpenChange={setCreating} />
    </div>
  )
}
