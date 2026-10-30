/** The “Referenced by” list of goal nodes for one canon row (spec/11 §5.5). */
import { Link } from "react-router"
import { useTranslation } from "react-i18next"

import { useBook } from "@/api/hooks/books"
import { useGoals } from "@/api/hooks/goals"
import type { GoalQuery } from "@/api/gen/GoalQuery"

export type GoalsReferencedByProps = {
  bookId: number
  entityId?: number
  eventId?: number
  foreshadowingId?: number
}

/** Hidden while Goals is off; the node links open the Goals page at the node. */
export function GoalsReferencedBy({
  bookId,
  entityId,
  eventId,
  foreshadowingId,
}: GoalsReferencedByProps) {
  const { t } = useTranslation()
  const book = useBook(bookId)
  const enabled = book.data?.settings.canon_systems.goals === true
  const query: GoalQuery = {}
  if (entityId !== undefined) query.entity_id = entityId
  if (eventId !== undefined) query.event_id = eventId
  if (foreshadowingId !== undefined) query.foreshadowing_id = foreshadowingId
  const goals = useGoals(bookId, query, enabled)
  if (!enabled || goals.data === undefined || goals.data.length === 0) return null
  return (
    <section className="space-y-1">
      <h3 className="text-xs font-medium text-muted-foreground">{t("canon:goal_referenced_by")}</h3>
      <ul className="flex flex-wrap gap-1">
        {goals.data.map((node) => (
          <li key={node.id}>
            <Link
              to={`/books/${bookId}/canon/goals?node=${node.id}`}
              className="rounded-full bg-muted px-2 py-0.5 text-xs underline-offset-2 hover:underline"
            >
              {node.title}
            </Link>
          </li>
        ))}
      </ul>
    </section>
  )
}
