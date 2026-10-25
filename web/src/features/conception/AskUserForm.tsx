/**
 * The `ask_user` form (PROJECT §8.7, spec/11 §5.3): one control group per question (radio for a
 * single choice, checkboxes when `multi_select`), an "Other" text field when the question allows
 * it, and a submit button.
 *
 * The payload is exactly what spec/05 §7 documents:
 * `{answers: [{id, selected: [label], other?}]}`. Labels are compared by their stored text (they
 * are model-facing, chosen by the prompt pack), so nothing is translated here.
 */
import { useState } from "react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"

/** One question of the recorded `ask_user` call (`ui.questions`, spec/05 §7). */
export type AskUserQuestion = {
  id: string
  question: string
  options: { label: string; description?: string }[]
  multi_select: boolean
  allow_other: boolean
}

/** Reads the recorded questions out of a tool call's `ui` payload. */
export function readQuestions(ui: unknown): AskUserQuestion[] {
  if (typeof ui !== "object" || ui === null) {
    return []
  }
  const raw = (ui as { questions?: unknown }).questions
  if (!Array.isArray(raw)) {
    return []
  }
  return raw.flatMap((item): AskUserQuestion[] => {
    if (typeof item !== "object" || item === null) {
      return []
    }
    const question = item as Record<string, unknown>
    const id = typeof question.id === "string" ? question.id : null
    const text = typeof question.question === "string" ? question.question : null
    if (id === null || text === null) {
      return []
    }
    const options = Array.isArray(question.options)
      ? question.options.flatMap((option): { label: string; description?: string }[] => {
          if (typeof option !== "object" || option === null) {
            return []
          }
          const entry = option as Record<string, unknown>
          const label = typeof entry.label === "string" ? entry.label : null
          if (label === null) {
            return []
          }
          return [
            {
              label,
              description:
                typeof entry.description === "string" && entry.description !== ""
                  ? entry.description
                  : undefined,
            },
          ]
        })
      : []
    return [
      {
        id,
        question: text,
        options,
        multi_select: question.multi_select === true,
        allow_other: question.allow_other !== false,
      },
    ]
  })
}

/** The answer payload of the form, as `POST /sessions/:id/answer` expects it (spec/05 §7). */
export function buildAnswerPayload(
  questions: AskUserQuestion[],
  state: { selected: Record<string, string[]>; other: Record<string, string> },
): { answers: { id: string; selected: string[]; other?: string }[] } {
  return {
    answers: questions.map((question) => {
      const other = (state.other[question.id] ?? "").trim()
      return {
        id: question.id,
        selected: state.selected[question.id] ?? [],
        ...(other === "" ? {} : { other }),
      }
    }),
  }
}

export function AskUserForm({
  questions,
  pending,
  onSubmit,
}: {
  questions: AskUserQuestion[]
  pending: boolean
  onSubmit: (payload: ReturnType<typeof buildAnswerPayload>) => void
}) {
  const { t } = useTranslation()
  const [selected, setSelected] = useState<Record<string, string[]>>({})
  const [other, setOther] = useState<Record<string, string>>({})

  const toggle = (question: AskUserQuestion, label: string, checked: boolean) => {
    setSelected((previous) => {
      const current = previous[question.id] ?? []
      const next = question.multi_select
        ? checked
          ? [...current, label]
          : current.filter((entry) => entry !== label)
        : checked
          ? [label]
          : []
      return { ...previous, [question.id]: next }
    })
  }

  return (
    <div className="space-y-4 rounded-md border border-sky-500/40 bg-sky-500/5 p-3">
      {questions.map((question) => (
        <fieldset key={question.id} className="space-y-2">
          <legend className="text-sm font-medium">{question.question}</legend>
          {question.multi_select ? (
            <div className="space-y-1.5">
              {question.options.map((option) => (
                <div key={option.label} className="flex items-start gap-2">
                  <Checkbox
                    id={`${question.id}-${option.label}`}
                    checked={(selected[question.id] ?? []).includes(option.label)}
                    onCheckedChange={(checked) =>
                      toggle(question, option.label, checked === true)
                    }
                  />
                  <div className="grid gap-0.5 leading-none">
                    <Label htmlFor={`${question.id}-${option.label}`} className="text-sm">
                      {option.label}
                    </Label>
                    {option.description === undefined ? null : (
                      <span className="text-xs text-muted-foreground">{option.description}</span>
                    )}
                  </div>
                </div>
              ))}
            </div>
          ) : question.options.length === 0 ? null : (
            <RadioGroup
              value={(selected[question.id] ?? [])[0] ?? ""}
              onValueChange={(value) => toggle(question, value, true)}
            >
              {question.options.map((option) => (
                <div key={option.label} className="flex items-start gap-2">
                  <RadioGroupItem id={`${question.id}-${option.label}`} value={option.label} />
                  <div className="grid gap-0.5 leading-none">
                    <Label htmlFor={`${question.id}-${option.label}`} className="text-sm">
                      {option.label}
                    </Label>
                    {option.description === undefined ? null : (
                      <span className="text-xs text-muted-foreground">{option.description}</span>
                    )}
                  </div>
                </div>
              ))}
            </RadioGroup>
          )}
          {question.allow_other ? (
            <Input
              value={other[question.id] ?? ""}
              placeholder={t("book:conception.other_placeholder")}
              onChange={(event) =>
                setOther((previous) => ({ ...previous, [question.id]: event.target.value }))
              }
            />
          ) : null}
        </fieldset>
      ))}
      <Button
        size="sm"
        disabled={pending}
        onClick={() => onSubmit(buildAnswerPayload(questions, { selected, other }))}
      >
        {pending ? t("common:saving") : t("book:conception.submit_answers")}
      </Button>
    </div>
  )
}
