import { type SubmitEvent, useState } from "react"
import { useTranslation } from "react-i18next"

import type { Question } from "@/api/generated/Question"
import type { QuestionView } from "@/api/generated/QuestionView"
import type { WorkspaceView } from "@/api/generated/WorkspaceView"
import { useAnswerQuestions } from "@/api/queries/workspace"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { Textarea } from "@/components/ui/textarea"

interface Draft {
  selected: string[]
  text: string
}

/**
 * The agent's questions (§8.6, §18.3): single and multiple choices, which also take the
 * user's own words, and free answers. The selections and the text are sent apart.
 */
export function QuestionDock({ view, question }: { view: WorkspaceView; question: QuestionView }) {
  const { t } = useTranslation()
  const answer = useAnswerQuestions(view.version.id)
  const [drafts, setDrafts] = useState<Record<string, Draft>>({})
  const draftOf = (id: string): Draft => drafts[id] ?? { selected: [], text: "" }

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    answer.mutate({
      callId: question.callId,
      answers: question.questions.map((item) => {
        const draft = draftOf(item.id)
        return {
          questionId: item.id,
          selectedOptionIds: draft.selected,
          customText: draft.text.trim() === "" ? null : draft.text,
        }
      }),
    })
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-4">
      <h3 className="font-medium">{t(($) => $.workspace.question.title)}</h3>
      {question.questions.map((item) => (
        <QuestionField
          key={item.id}
          callId={question.callId}
          question={item}
          draft={draftOf(item.id)}
          onChange={(draft) => setDrafts((current) => ({ ...current, [item.id]: draft }))}
        />
      ))}
      {answer.isError && <ErrorAlert error={answer.error} />}
      <div>
        <Button type="submit" disabled={answer.isPending}>
          {t(($) => $.workspace.question.submit)}
        </Button>
      </div>
    </form>
  )
}

function QuestionField({
  callId,
  question,
  draft,
  onChange,
}: {
  callId: string
  question: Question
  draft: Draft
  onChange: (draft: Draft) => void
}) {
  const { t } = useTranslation()
  const id = (part: string) => `question-${callId}-${question.id}-${part}`

  return (
    <fieldset className="flex flex-col gap-2">
      <legend className="mb-1 flex items-center gap-2 font-medium">
        {question.title}
        {question.required && (
          <Badge variant="outline">{t(($) => $.workspace.question.required)}</Badge>
        )}
      </legend>
      {question.kind === "single" && (
        <RadioGroup
          value={draft.selected[0] ?? null}
          onValueChange={(value: string) => onChange({ ...draft, selected: [value] })}
        >
          {question.options.map((option) => (
            <div key={option.id} className="flex items-center gap-2">
              <RadioGroupItem id={id(option.id)} value={option.id} />
              <Label htmlFor={id(option.id)}>{option.label}</Label>
            </div>
          ))}
        </RadioGroup>
      )}
      {question.kind === "multiple" &&
        question.options.map((option) => (
          <div key={option.id} className="flex items-center gap-2">
            <Checkbox
              id={id(option.id)}
              checked={draft.selected.includes(option.id)}
              onCheckedChange={(checked) =>
                onChange({
                  ...draft,
                  selected: checked
                    ? [...draft.selected, option.id]
                    : draft.selected.filter((selected) => selected !== option.id),
                })
              }
            />
            <Label htmlFor={id(option.id)}>{option.label}</Label>
          </div>
        ))}
      {question.kind === "text" ? (
        <>
          <Label htmlFor={id("text")} className="sr-only">
            {t(($) => $.workspace.question.answer)}
          </Label>
          <Textarea
            id={id("text")}
            value={draft.text}
            onChange={(event) => onChange({ ...draft, text: event.target.value })}
            rows={2}
          />
        </>
      ) : (
        <Input
          aria-label={t(($) => $.workspace.question.other)}
          placeholder={t(($) => $.workspace.question.other)}
          value={draft.text}
          onChange={(event) => onChange({ ...draft, text: event.target.value })}
        />
      )}
    </fieldset>
  )
}
