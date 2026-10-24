/** The feedback dialog: free text sent with `resume` on a held or paused session (spec/03 §6). */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Textarea } from "@/components/ui/textarea"

export function FeedbackDialog({
  open,
  onOpenChange,
  onSubmit,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onSubmit: (feedback: string) => void
}) {
  const { t } = useTranslation()
  // The text lives in the dialog's parent key: closing and reopening remounts it cleared, so no
  // effect has to reset the state (React Compiler forbids setState in effects).
  const [text, setText] = useState("")

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("agent:feedback.title")}</DialogTitle>
          <DialogDescription>{t("agent:feedback.description")}</DialogDescription>
        </DialogHeader>
        <Textarea
          value={text}
          onChange={(event) => setText(event.target.value)}
          placeholder={t("agent:feedback.placeholder")}
          rows={5}
          autoFocus
        />
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common:cancel")}
          </Button>
          <Button
            disabled={text.trim() === ""}
            onClick={() => {
              onSubmit(text.trim())
              setText("")
            }}
          >
            {t("agent:feedback.submit")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
