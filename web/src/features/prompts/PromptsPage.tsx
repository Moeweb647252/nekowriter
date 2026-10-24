/** The global prompt pack page (PROJECT §8.11, spec/09 §4): overrides for every book. */
import { useTranslation } from "react-i18next"

import { PromptEditor } from "@/features/prompts/PromptEditor"

export default function PromptsPage() {
  const { t } = useTranslation()
  return (
    <div className="h-full overflow-y-auto p-6">
      <div className="mb-4">
        <h1 className="text-lg font-semibold">{t("prompts:title")}</h1>
        <p className="text-sm text-muted-foreground">{t("prompts:hint")}</p>
      </div>
      <PromptEditor bookScope={false} />
    </div>
  )
}
