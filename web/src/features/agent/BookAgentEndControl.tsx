/** End / New conversation for the book agent (plan §3.7). */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { LogOut } from "lucide-react"

import { endSession } from "@/api/endpoints/sessions"
import type { SessionDto } from "@/api/gen/SessionDto"
import { Button } from "@/components/ui/button"
import { ConfirmDialog } from "@/components/common/ConfirmDialog"

export function BookAgentEndControl({
  session,
  onEnded,
}: {
  session: SessionDto
  onEnded: () => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [busy, setBusy] = useState(false)

  const end = async () => {
    setBusy(true)
    try {
      await endSession(session.id)
      setOpen(false)
      onEnded()
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <div className="flex shrink-0 gap-1 border-b px-3 py-2">
        <Button size="sm" variant="ghost" onClick={() => setOpen(true)}>
          <LogOut aria-hidden="true" />
          {t("agent:book_agent.end")}
        </Button>
        <Button size="sm" variant="ghost" onClick={() => setOpen(true)}>
          {t("agent:book_agent.new_conversation")}
        </Button>
      </div>
      <ConfirmDialog
        open={open}
        onOpenChange={setOpen}
        title={t("agent:book_agent.end_title")}
        description={
          session.status === "held"
            ? t("agent:book_agent.end_held_body")
            : t("agent:book_agent.end_body")
        }
        confirmLabel={t("agent:book_agent.end_confirm")}
        pending={busy}
        onConfirm={() => void end()}
      />
    </>
  )
}
