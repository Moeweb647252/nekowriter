/**
 * The research topics editor (PROJECT §8.7, spec/11 §5.4): rows of topic, requirements and target
 * scope, editable while the conception session is not running. Each row is a `draft` research
 * task; adding and removing rows edits the tasks through the research API (spec/10 §2.5).
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Plus, Trash2 } from "lucide-react"

import type { ResearchTaskDto } from "@/api/gen/ResearchTaskDto"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"

export function ResearchTopicsEditor({
  topics,
  editable,
  busy,
  onCreate,
  onPatch,
  onDelete,
}: {
  topics: ResearchTaskDto[]
  editable: boolean
  busy: boolean
  onCreate: (input: { topic: string; requirements: string; scope_name: string }) => void
  onPatch: (
    id: number,
    patch: { topic?: string; requirements?: string; scope_name?: string },
  ) => void
  onDelete: (id: number) => void
}) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState({ topic: "", requirements: "", scope: "" })

  return (
    <div className="space-y-2">
      {topics.length === 0 ? (
        <p className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
          {t("book:conception.no_topics")}
        </p>
      ) : (
        <ul className="space-y-1.5">
          {topics.map((topic) => (
            <TopicRow
              key={topic.id}
              topic={topic}
              editable={editable}
              busy={busy}
              onPatch={onPatch}
              onDelete={onDelete}
            />
          ))}
        </ul>
      )}
      {editable ? (
        <div className="space-y-2 rounded-md border border-dashed p-2">
          <div className="grid grid-cols-2 gap-2">
            <div className="space-y-1">
              <Label className="text-xs">{t("book:conception.topic")}</Label>
              <Input
                value={draft.topic}
                onChange={(event) =>
                  setDraft((previous) => ({ ...previous, topic: event.target.value }))
                }
              />
            </div>
            <div className="space-y-1">
              <Label className="text-xs">{t("book:conception.topic_scope")}</Label>
              <Input
                value={draft.scope}
                onChange={(event) =>
                  setDraft((previous) => ({ ...previous, scope: event.target.value }))
                }
              />
            </div>
          </div>
          <div className="space-y-1">
            <Label className="text-xs">{t("book:conception.topic_requirements")}</Label>
            <Input
              value={draft.requirements}
              onChange={(event) =>
                setDraft((previous) => ({ ...previous, requirements: event.target.value }))
              }
            />
          </div>
          <Button
            size="sm"
            variant="outline"
            disabled={
              busy ||
              draft.topic.trim() === "" ||
              draft.scope.trim() === ""
            }
            onClick={() => {
              onCreate({
                topic: draft.topic.trim(),
                requirements: draft.requirements.trim(),
                scope_name: draft.scope.trim(),
              })
              setDraft({ topic: "", requirements: "", scope: "" })
            }}
          >
            <Plus />
            {t("book:conception.add_topic")}
          </Button>
        </div>
      ) : null}
    </div>
  )
}

function TopicRow({
  topic,
  editable,
  busy,
  onPatch,
  onDelete,
}: {
  topic: ResearchTaskDto
  editable: boolean
  busy: boolean
  onPatch: (
    id: number,
    patch: { topic?: string; requirements?: string; scope_name?: string },
  ) => void
  onDelete: (id: number) => void
}) {
  const { t } = useTranslation()
  const [state, setState] = useState({
    topic: topic.topic,
    requirements: topic.requirements,
    scope_name: topic.scope_name,
  })
  const dirty =
    state.topic !== topic.topic ||
    state.requirements !== topic.requirements ||
    state.scope_name !== topic.scope_name

  return (
    <li className="space-y-1.5 rounded-md border p-2">
      <div className="flex items-center gap-2">
        <Input
          value={state.topic}
          disabled={!editable}
          className="flex-1"
          onChange={(event) =>
            setState((previous) => ({ ...previous, topic: event.target.value }))
          }
        />
        <Input
          value={state.scope_name}
          disabled={!editable}
          className="w-32"
          aria-label={t("book:conception.topic_scope")}
          onChange={(event) =>
            setState((previous) => ({ ...previous, scope_name: event.target.value }))
          }
        />
        {topic.origin === "author" ? (
          <Badge variant="outline" data-testid="added-by-you" className="shrink-0 text-[0.65rem]">
            {t("book:conception.added_by_you")}
          </Badge>
        ) : null}
        {editable ? (
          <>
            <Button
              size="sm"
              variant="outline"
              disabled={!dirty || busy || state.topic.trim() === "" || state.scope_name.trim() === ""}
              onClick={() =>
                onPatch(topic.id, {
                  topic: state.topic.trim(),
                  requirements: state.requirements.trim(),
                  scope_name: state.scope_name.trim(),
                })
              }
            >
              {t("common:save")}
            </Button>
            <Button
              size="icon-xs"
              variant="ghost"
              aria-label={t("common:delete")}
              disabled={busy}
              onClick={() => onDelete(topic.id)}
            >
              <Trash2 />
            </Button>
          </>
        ) : null}
      </div>
      <Input
        value={state.requirements}
        disabled={!editable}
        placeholder={t("book:conception.topic_requirements")}
        onChange={(event) =>
          setState((previous) => ({ ...previous, requirements: event.target.value }))
        }
      />
    </li>
  )
}
