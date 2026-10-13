import { ArchiveIcon, BookOpenIcon } from "lucide-react"
import { useTranslation } from "react-i18next"

import { ProjectCard } from "./ProjectCard"
import type { ProjectListFilter } from "@/api/generated/ProjectListFilter"
import { useProjects } from "@/api/queries/projects"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty"
import { Skeleton } from "@/components/ui/skeleton"

export function ProjectList({ filter }: { filter: ProjectListFilter }) {
  const projects = useProjects(filter)

  if (projects.isPending) {
    return (
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3" aria-hidden>
        {[0, 1, 2].map((index) => (
          <Skeleton key={index} className="h-36 rounded-xl" />
        ))}
      </div>
    )
  }
  if (projects.isError) {
    return <ErrorAlert error={projects.error} onRetry={() => void projects.refetch()} />
  }
  if (projects.data.length === 0) {
    return <EmptyLibrary filter={filter} />
  }
  return (
    <ul className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {projects.data.map((project) => (
        <li key={project.id}>
          <ProjectCard project={project} />
        </li>
      ))}
    </ul>
  )
}

function EmptyLibrary({ filter }: { filter: ProjectListFilter }) {
  const { t } = useTranslation()
  const deleted = filter === "deleted"
  return (
    <Empty className="border">
      <EmptyHeader>
        <EmptyMedia variant="icon">{deleted ? <ArchiveIcon /> : <BookOpenIcon />}</EmptyMedia>
        <EmptyTitle>
          {deleted ? t(($) => $.library.empty.deletedTitle) : t(($) => $.library.empty.activeTitle)}
        </EmptyTitle>
        <EmptyDescription>
          {deleted ? t(($) => $.library.empty.deletedBody) : t(($) => $.library.empty.activeBody)}
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
  )
}
