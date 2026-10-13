import { PlusIcon } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { CreateProjectDialog } from "./CreateProjectDialog"
import { ProjectList } from "./ProjectList"
import type { ProjectListFilter } from "@/api/generated/ProjectListFilter"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"

const FILTERS = ["active", "deleted"] as const satisfies readonly ProjectListFilter[]

/** The library of works: the active ones, the deleted ones, and creating new ones. */
export function WorksLibraryPage() {
  const { t } = useTranslation()
  const [filter, setFilter] = useState<ProjectListFilter>("active")
  const [creating, setCreating] = useState(false)

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-center justify-between gap-4">
        <h1 className="font-heading text-2xl font-semibold">{t(($) => $.library.title)}</h1>
        <Button onClick={() => setCreating(true)}>
          <PlusIcon />
          {t(($) => $.library.newWork)}
        </Button>
      </div>
      <Tabs value={filter} onValueChange={(value: ProjectListFilter) => setFilter(value)}>
        <TabsList aria-label={t(($) => $.library.filterLabel)}>
          {FILTERS.map((value) => (
            <TabsTrigger key={value} value={value}>
              {t(($) => $.common.projectFilter[value])}
            </TabsTrigger>
          ))}
        </TabsList>
        {FILTERS.map((value) => (
          <TabsContent key={value} value={value} className="pt-4">
            <ProjectList filter={value} />
          </TabsContent>
        ))}
      </Tabs>
      <CreateProjectDialog open={creating} onOpenChange={setCreating} />
    </div>
  )
}
