/** A confirmation dialog for destructive actions (PROJECT §8.13). */
import type * as React from "react"
import { useTranslation } from "react-i18next"

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"

export type ConfirmDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description: string
  /** The label of the confirming button. */
  confirmLabel?: string
  onConfirm: () => void
  destructive?: boolean
  pending?: boolean
  /** When set, the author must type exactly this text (deleting a book, PROJECT §8.3). */
  confirmText?: string
  typed?: string
  onTypedChange?: (value: string) => void
}

/** A dialog that asks once before destroying something. */
export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  onConfirm,
  destructive = true,
  pending = false,
  confirmText,
  typed = "",
  onTypedChange,
}: ConfirmDialogProps) {
  const { t } = useTranslation()
  const matches = confirmText === undefined || typed.trim() === confirmText
  const children: React.ReactNode = confirmText === undefined ? null : (
    <div className="mt-2 space-y-1.5">
      <Label htmlFor="confirm-typed" className="text-xs text-muted-foreground">
        {t("common:type_to_confirm", { text: confirmText })}
      </Label>
      <Input
        id="confirm-typed"
        value={typed}
        onChange={(event) => onTypedChange?.(event.target.value)}
        autoComplete="off"
      />
    </div>
  )
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription>{description}</AlertDialogDescription>
        </AlertDialogHeader>
        {children}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={pending}>{t("common:cancel")}</AlertDialogCancel>
          <AlertDialogAction
            variant={destructive ? "destructive" : "default"}
            disabled={pending || !matches}
            onClick={(event) => {
              event.preventDefault()
              onConfirm()
            }}
          >
            {confirmLabel ?? t("common:confirm")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
