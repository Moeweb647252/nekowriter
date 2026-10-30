/** The banner a disabled canon page keeps above its data (spec/11 §5.5, spec/06 §10.3). */
import { useQueryClient } from "@tanstack/react-query"
import { useTranslation } from "react-i18next"

import { usePatchBook } from "@/api/hooks/books"
import { keys } from "@/api/keys"
import { Button } from "@/components/ui/button"
import type { BookDto } from "@/api/gen/BookDto"
import type { CanonPageSystem } from "@/features/workspace/canonVisibility"

export function SystemOffBanner({
  book,
  system,
}: {
  book: BookDto
  system: CanonPageSystem
}) {
  const { t } = useTranslation()
  const patch = usePatchBook(book.id)
  const queryClient = useQueryClient()
  if (book.settings.canon_systems[system]) return null

  const turnOn = () => {
    const settings = book.settings
    patch.mutate(
      {
        settings: {
          ...settings,
          canon_systems: { ...settings.canon_systems, [system]: true },
        },
      },
      {
        onSuccess: () => {
          void queryClient.invalidateQueries({ queryKey: keys.canon.all })
        },
      },
    )
  }

  return (
    <div
      role="status"
      data-testid={`system-off-${system}`}
      className="flex flex-wrap items-center justify-between gap-2 border-b border-dashed bg-muted/30 px-3 py-2 text-xs text-muted-foreground"
    >
      <span>{t("book:system_off_banner")}</span>
      <Button size="sm" variant="outline" disabled={patch.isPending} onClick={turnOn}>
        {t("book:system_turn_on")}
      </Button>
    </div>
  )
}
