import { useState, type FormEvent } from "react"
import { LoaderCircle, MessageCircleQuestion, Send } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  answerQuestion,
  type DecimalString,
  type PendingUserQuestion,
  type UserQuestionAnswer,
} from "@/lib/api"

type AnswerDraft = {
  selectedOptionIds: string[]
  customText: string
}

type UserQuestionPanelProps = {
  question: PendingUserQuestion
  stateRevision: DecimalString
  disabled?: boolean
  onAnswered: () => Promise<void>
  onError: (message: string) => void
}

function initialAnswers(question: PendingUserQuestion) {
  return Object.fromEntries(
    question.schema.questions.map((item) => [
      item.id,
      { selectedOptionIds: [], customText: "" } satisfies AnswerDraft,
    ]),
  ) as Record<string, AnswerDraft>
}

export function UserQuestionPanel({
  question,
  stateRevision,
  disabled = false,
  onAnswered,
  onError,
}: UserQuestionPanelProps) {
  const [answers, setAnswers] = useState<Record<string, AnswerDraft>>(() => initialAnswers(question))
  const [submitting, setSubmitting] = useState(false)

  function updateAnswer(questionId: string, update: Partial<AnswerDraft>) {
    setAnswers((current) => ({
      ...current,
      [questionId]: { ...current[questionId], ...update },
    }))
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setSubmitting(true)
    try {
      const payload: UserQuestionAnswer[] = question.schema.questions.map((item) => {
        const draft = answers[item.id] ?? { selectedOptionIds: [], customText: "" }
        return {
          questionId: item.id,
          selectedOptionIds: draft.selectedOptionIds,
          customText: draft.customText.length > 0 ? draft.customText : null,
        }
      })
      await answerQuestion(question.id, stateRevision, payload)
      await onAnswered()
    } catch (error) {
      onError(error instanceof Error ? error.message : "问题答案未能保存")
    } finally {
      setSubmitting(false)
    }
  }

  const locked = disabled || submitting

  return (
    <form
      className="mt-5 rounded-xl border border-primary/30 bg-primary/5 p-4"
      onSubmit={handleSubmit}
    >
      <div className="flex items-start gap-3">
        <MessageCircleQuestion className="mt-0.5 size-5 shrink-0 text-primary" aria-hidden="true" />
        <div>
          <h3 className="text-sm font-semibold">继续前需要你的回答</h3>
          <p className="mt-1 text-sm leading-6 text-muted-foreground">
            这些答案会作为本次 Agent 工具调用的正式结果回送。自由意见不会替代这里的回答。
          </p>
        </div>
      </div>

      <div className="mt-4 space-y-5">
        {question.schema.questions.map((item, index) => {
          const answer = answers[item.id] ?? { selectedOptionIds: [], customText: "" }
          const fieldId = `question-${question.id}-${item.id}`
          return (
            <fieldset key={item.id} className="rounded-lg border border-border bg-background/75 p-4">
              <legend className="px-1 text-sm font-medium">
                {index + 1}. {item.title}
                {item.required && <span className="ml-1 text-destructive">*</span>}
              </legend>

              {item.kind === "single" && (
                <div className="mt-3 space-y-2">
                  {item.options.map((option) => {
                    const optionId = `${fieldId}-${option.id}`
                    return (
                      <label key={option.id} htmlFor={optionId} className="flex cursor-pointer items-start gap-2 text-sm leading-6">
                        <input
                          id={optionId}
                          type="radio"
                          name={fieldId}
                          checked={answer.selectedOptionIds[0] === option.id}
                          disabled={locked}
                          onChange={() => updateAnswer(item.id, { selectedOptionIds: [option.id] })}
                          className="mt-1 size-4 accent-primary"
                        />
                        <span>{option.label}</span>
                      </label>
                    )
                  })}
                </div>
              )}

              {item.kind === "multiple" && (
                <div className="mt-3 space-y-2">
                  {item.options.map((option) => {
                    const optionId = `${fieldId}-${option.id}`
                    const selected = answer.selectedOptionIds.includes(option.id)
                    return (
                      <label key={option.id} htmlFor={optionId} className="flex cursor-pointer items-start gap-2 text-sm leading-6">
                        <input
                          id={optionId}
                          type="checkbox"
                          checked={selected}
                          disabled={locked}
                          onChange={() =>
                            updateAnswer(item.id, {
                              selectedOptionIds: selected
                                ? answer.selectedOptionIds.filter((id) => id !== option.id)
                                : [...answer.selectedOptionIds, option.id],
                            })
                          }
                          className="mt-1 size-4 rounded accent-primary"
                        />
                        <span>{option.label}</span>
                      </label>
                    )
                  })}
                </div>
              )}

              <label className="mt-3 block text-xs font-medium text-muted-foreground" htmlFor={`${fieldId}-custom`}>
                {item.kind === "text" ? "你的回答" : "补充说明（可选）"}
                <textarea
                  id={`${fieldId}-custom`}
                  value={answer.customText}
                  onChange={(event) => updateAnswer(item.id, { customText: event.target.value })}
                  disabled={locked}
                  maxLength={20_000}
                  required={item.kind === "text" && item.required}
                  placeholder={item.kind === "text" ? "输入你的回答" : "可补充未列出的选择或背景"}
                  className="mt-1.5 min-h-24 w-full resize-y rounded-lg border border-input bg-background px-3 py-2 text-sm leading-6 text-foreground outline-none transition placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/20 disabled:cursor-not-allowed disabled:opacity-60"
                />
              </label>
            </fieldset>
          )
        })}
      </div>

      <div className="mt-4 flex flex-wrap items-center justify-between gap-3">
        <p className="text-xs leading-5 text-muted-foreground">带 * 的问题必须至少选择一项或填写回答。</p>
        <Button type="submit" disabled={locked}>
          {submitting ? <LoaderCircle className="animate-spin" aria-hidden="true" /> : <Send aria-hidden="true" />}
          提交回答并继续
        </Button>
      </div>
    </form>
  )
}
