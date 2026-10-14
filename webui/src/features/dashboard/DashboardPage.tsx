import { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"

import { JobsCard } from "./JobsCard"
import { type DashboardRange, RANGES, bucketBoundaries } from "./range"
import { UsageCard } from "./UsageCard"
import { WordsCard } from "./WordsCard"
import type { ProjectId } from "@/api/generated/ProjectId"
import { useDashboard } from "@/api/queries/dashboard"
import { useProjects } from "@/api/queries/projects"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useFormatters } from "@/i18n/format"

/** Token usage, running jobs and word-count growth, for all works or one (§15). */
export function DashboardPage() {
  const { t } = useTranslation()
  const format = useFormatters()
  const [range, setRange] = useState<DashboardRange>("week")
  const [project, setProject] = useState<ProjectId | null>(null)
  // The buckets follow the calendar day, so a page left open moves on at midnight.
  const day = new Date().toDateString()
  const boundaries = useMemo(() => bucketBoundaries(range, new Date(day)), [range, day])
  const dashboard = useDashboard(boundaries, project)
  const projects = useProjects("active")

  const bucketLabel = (index: number) => {
    const start = boundaries[index] ?? 0
    return range === "today" ? format.hour(start) : format.day(start)
  }
  const projectItems = [
    { value: null, label: t(($) => $.dashboard.allWorks) },
    ...(projects.data ?? []).map((item) => ({ value: item.id, label: item.name })),
  ]

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="mr-auto font-heading text-2xl font-semibold">
          {t(($) => $.dashboard.title)}
        </h1>
        <Select
          items={projectItems}
          value={project}
          onValueChange={(value: ProjectId | null) => setProject(value)}
        >
          <SelectTrigger aria-label={t(($) => $.dashboard.work)} className="w-48">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {projectItems.map((item) => (
              <SelectItem key={item.value ?? "all"} value={item.value}>
                {item.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Tabs value={range} onValueChange={(value: DashboardRange) => setRange(value)}>
          <TabsList aria-label={t(($) => $.dashboard.range)}>
            {RANGES.map((item) => (
              <TabsTrigger key={item} value={item}>
                {t(($) => $.dashboard.ranges[item])}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      </div>
      {dashboard.isPending && <Skeleton className="h-64" />}
      {dashboard.isError && (
        <ErrorAlert error={dashboard.error} onRetry={() => void dashboard.refetch()} />
      )}
      {dashboard.isSuccess && (
        <>
          <JobsCard jobs={dashboard.data.jobs} attention={dashboard.data.attention} />
          <WordsCard words={dashboard.data.words} bucketLabel={bucketLabel} />
          <UsageCard usage={dashboard.data.usage} bucketLabel={bucketLabel} />
        </>
      )}
    </div>
  )
}
