/**
 * The shared "previous chapter is not ready" block of the Outline and Write dialogs
 * (spec/10 §3 `chapter.previous_not_ready`, spec/11 §5.2, D-146).
 */
import { useTranslation } from "react-i18next"

/** The state the API reported in `409 chapter.previous_not_ready`. */
export type PreviousNotReady = {
  chapterNumber?: number
  reason?: string
}

export function PreviousNotReadyNotice({ notReady }: { notReady: PreviousNotReady }) {
  const { t } = useTranslation()
  return (
    <div className="space-y-2 rounded-md border border-status-waiting/40 bg-status-waiting/10 p-3 text-sm">
      <p className="font-medium">{t("chapter:write_dialog.not_ready_title")}</p>
      <p className="text-muted-foreground">
        {notReady.reason === "canon_not_synced"
          ? t("chapter:write_dialog.not_ready_canon", { number: notReady.chapterNumber ?? 0 })
          : t("chapter:write_dialog.not_ready_status", { number: notReady.chapterNumber ?? 0 })}
      </p>
    </div>
  )
}
