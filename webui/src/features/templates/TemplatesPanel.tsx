import { PlusIcon } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { TemplateForm } from "./TemplateForm"
import type { PromptTemplateId } from "@/api/generated/PromptTemplateId"
import { useTemplates } from "@/api/queries/templates"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"

/** The user's prompt templates: a list and the selected one's form (§13.2). */
export function TemplatesPanel() {
  const { t } = useTranslation()
  const templates = useTemplates()
  // `null` edits a new template.
  const [selected, setSelected] = useState<PromptTemplateId | null>(null)
  const template = templates.data?.find((item) => item.id === selected)

  if (templates.isPending) return <Skeleton className="h-96" />
  if (templates.isError) {
    return <ErrorAlert error={templates.error} onRetry={() => void templates.refetch()} />
  }
  return (
    <div className="flex flex-col gap-4">
      <p className="max-w-2xl text-sm text-muted-foreground">{t(($) => $.templates.description)}</p>
      <div className="grid gap-6 md:grid-cols-[14rem_1fr]">
        <nav aria-label={t(($) => $.templates.list)} className="flex flex-col gap-2">
          <Button variant="outline" size="sm" onClick={() => setSelected(null)}>
            <PlusIcon />
            {t(($) => $.templates.new)}
          </Button>
          {templates.data.length === 0 ? (
            <p className="px-2 text-sm text-muted-foreground">{t(($) => $.templates.empty)}</p>
          ) : (
            <ul className="flex flex-col">
              {templates.data.map((item) => (
                <li key={item.id}>
                  <button
                    type="button"
                    aria-current={item.id === selected ? "true" : undefined}
                    onClick={() => setSelected(item.id)}
                    className={cn(
                      "w-full truncate rounded-md px-2 py-1.5 text-left text-sm text-muted-foreground hover:text-foreground",
                      item.id === selected && "bg-muted text-foreground",
                    )}
                  >
                    {item.title}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </nav>
        <TemplateForm
          // Each template, and the new one, gets a fresh form.
          key={template?.id ?? "new"}
          template={template ?? null}
          onSaved={setSelected}
          onDeleted={() => setSelected(null)}
        />
      </div>
    </div>
  )
}
