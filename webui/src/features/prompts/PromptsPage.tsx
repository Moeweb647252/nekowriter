import { useState } from "react"
import { useTranslation } from "react-i18next"

import { PromptEditor } from "./PromptEditor"
import { TemplatesPanel } from "@/features/templates/TemplatesPanel"
import type { ProjectId } from "@/api/generated/ProjectId"
import type { PromptKey } from "@/api/generated/PromptKey"
import type { PromptRole } from "@/api/generated/PromptRole"
import { useProjects } from "@/api/queries/projects"
import { usePrompts } from "@/api/queries/prompts"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { cn } from "@/lib/utils"

const GROUPS = {
  writing: ["preparation.generate", "chapter.outline", "chapter.write"],
  review: [
    "review.preparation.background",
    "review.preparation.research",
    "review.preparation.characters",
    "review.preparation.outline",
    "review.chapter.outline",
    "review.chapter.edit",
    "review.chapter.final",
  ],
} as const satisfies Record<string, readonly PromptKey[]>

const ROLES: readonly PromptRole[] = ["system", "user"]

/** The built-in prompts with the system's and each work's overrides (§13.1). */
export function PromptsPage() {
  const { t } = useTranslation()
  const [project, setProject] = useState<ProjectId | null>(null)
  const [selected, setSelected] = useState<PromptKey>("preparation.generate")
  const projects = useProjects("active")
  const prompts = usePrompts(project)

  const scopeItems = [
    { value: null, label: t(($) => $.prompts.allWorks) },
    ...(projects.data ?? []).map((item) => ({ value: item.id, label: item.name })),
  ]

  return (
    <div className="flex flex-col gap-6">
      <div className="flex max-w-2xl flex-col gap-1">
        <h1 className="font-heading text-2xl font-semibold">{t(($) => $.prompts.title)}</h1>
        <p className="text-sm text-muted-foreground">{t(($) => $.prompts.description)}</p>
      </div>
      <Tabs defaultValue="agents">
        <TabsList>
          <TabsTrigger value="agents">{t(($) => $.prompts.tabs.agents)}</TabsTrigger>
          <TabsTrigger value="templates">{t(($) => $.prompts.tabs.templates)}</TabsTrigger>
        </TabsList>
        <TabsContent value="agents" className="flex flex-col gap-4 pt-4">
          <Select
            items={scopeItems}
            value={project}
            onValueChange={(value: ProjectId | null) => setProject(value)}
          >
            <SelectTrigger aria-label={t(($) => $.prompts.scope)} className="w-56">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {scopeItems.map((item) => (
                <SelectItem key={item.value ?? "system"} value={item.value}>
                  {item.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <div className="grid gap-6 md:grid-cols-[14rem_1fr]">
            <PromptList selected={selected} onSelect={setSelected} />
            <section aria-label={t(($) => $.prompts.keys[selected])}>
              {prompts.isPending && <Skeleton className="h-96" />}
              {prompts.isError && (
                <ErrorAlert error={prompts.error} onRetry={() => void prompts.refetch()} />
              )}
              {prompts.isSuccess && (
                <Tabs defaultValue="system">
                  <TabsList>
                    {ROLES.map((role) => (
                      <TabsTrigger key={role} value={role}>
                        {t(($) => $.prompts.roles[role])}
                      </TabsTrigger>
                    ))}
                  </TabsList>
                  {ROLES.map((role) => {
                    const prompt = prompts.data.prompts.find(
                      (item) => item.key === selected && item.role === role,
                    )
                    return (
                      prompt !== undefined && (
                        <TabsContent key={role} value={role} className="pt-4">
                          <PromptEditor
                            // A new prompt or scope starts a new editor, with its own draft.
                            key={`${project ?? "system"}.${selected}.${role}`}
                            view={prompts.data}
                            prompt={prompt}
                            project={project}
                          />
                        </TabsContent>
                      )
                    )
                  })}
                </Tabs>
              )}
            </section>
          </div>
        </TabsContent>
        <TabsContent value="templates" className="pt-4">
          <TemplatesPanel />
        </TabsContent>
      </Tabs>
    </div>
  )
}

function PromptList({
  selected,
  onSelect,
}: {
  selected: PromptKey
  onSelect: (key: PromptKey) => void
}) {
  const { t } = useTranslation()
  return (
    <nav aria-label={t(($) => $.prompts.list)} className="flex flex-col gap-4">
      {(["writing", "review"] as const).map((group) => (
        <section key={group}>
          <h2 className="mb-1 px-2 text-xs font-medium text-muted-foreground uppercase">
            {t(($) => $.prompts.groups[group])}
          </h2>
          <ul className="flex flex-col">
            {GROUPS[group].map((key) => (
              <li key={key}>
                <button
                  type="button"
                  aria-current={key === selected ? "true" : undefined}
                  onClick={() => onSelect(key)}
                  className={cn(
                    "w-full rounded-md px-2 py-1.5 text-left text-sm text-muted-foreground hover:text-foreground",
                    key === selected && "bg-muted text-foreground",
                  )}
                >
                  {t(($) => $.prompts.keys[key])}
                </button>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </nav>
  )
}
