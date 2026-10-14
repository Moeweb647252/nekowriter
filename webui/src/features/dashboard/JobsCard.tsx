import { useTranslation } from "react-i18next"
import { Link } from "react-router"

import type { JobView } from "@/api/generated/JobView"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { useChapterName } from "@/features/workspace/chapters"
import { useHaltText } from "@/features/workspace/run"
import { workspacePath } from "@/features/workspace/stage"

/** The versions' current runs: working or waiting, and those that need the user. */
export function JobsCard({
  jobs,
  attention,
}: {
  jobs: readonly JobView[]
  attention: readonly JobView[]
}) {
  const { t } = useTranslation()
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t(($) => $.dashboard.jobs.title)}</CardTitle>
      </CardHeader>
      <CardContent className="grid gap-6 md:grid-cols-2">
        <JobList
          title={t(($) => $.dashboard.jobs.running)}
          empty={t(($) => $.dashboard.jobs.noneRunning)}
          jobs={jobs}
        />
        <JobList
          title={t(($) => $.dashboard.jobs.attention)}
          empty={t(($) => $.dashboard.jobs.noneAttention)}
          jobs={attention}
        />
      </CardContent>
    </Card>
  )
}

function JobList({
  title,
  empty,
  jobs,
}: {
  title: string
  empty: string
  jobs: readonly JobView[]
}) {
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-sm font-medium">{title}</h3>
      {jobs.length === 0 ? (
        <p className="text-sm text-muted-foreground">{empty}</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {jobs.map((job) => (
            <Job key={job.runId} job={job} />
          ))}
        </ul>
      )}
    </section>
  )
}

function Job({ job }: { job: JobView }) {
  const { t } = useTranslation()
  const chapterName = useChapterName()
  const haltText = useHaltText()
  const task = t(($) => $.dashboard.jobs.kind[job.kind])
  const reviewAgent = job.reviewAgent
  return (
    <li className="rounded-md border px-3 py-2 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <Link
          to={workspacePath(job.projectId, job.versionId)}
          className="font-medium underline-offset-4 hover:underline"
        >
          {t(($) => $.dashboard.jobs.where, {
            work: job.projectName,
            version: job.versionNumber,
          })}
        </Link>
        <Badge variant="secondary">{t(($) => $.workspace.run.status[job.status])}</Badge>
      </div>
      <p className="text-muted-foreground">
        {job.chapter === null ? task : `${task} · ${chapterName(job.chapter)}`}
      </p>
      {reviewAgent !== null && (
        <p className="text-muted-foreground">
          {t(($) => $.dashboard.jobs.reviewAgent, {
            status: t(($) => $.workspace.run.status[reviewAgent]),
          })}
        </p>
      )}
      {job.halt !== null && <p className="mt-1">{haltText(job.halt)}</p>}
    </li>
  )
}
