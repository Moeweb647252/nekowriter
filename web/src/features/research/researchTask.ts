/** Shared presentation rules for research tasks (spec/11 §5.5, D-158). */
import type { ResearchTaskDto } from "@/api/gen/ResearchTaskDto"

/** The latest knowledge check of a book, from the research tasks a page already loads. */
export function latestPreparationTask(tasks: ResearchTaskDto[]): ResearchTaskDto | undefined {
  return tasks
    .filter((task) => task.kind === "preparation")
    .reduce<ResearchTaskDto | undefined>(
      (latest, task) => (latest === undefined || task.id > latest.id ? task : latest),
      undefined,
    )
}

/** A knowledge check is open while its task has not reached a terminal state. */
export function knowledgeCheckOpen(task: ResearchTaskDto | undefined): boolean {
  return task !== undefined && !["done", "failed", "cancelled"].includes(task.status)
}
