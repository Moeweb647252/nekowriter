import { useTranslation } from "react-i18next"

import { errorForField } from "@/api/fieldErrors"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldError,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { useErrorText } from "@/i18n/errors"

import type { ProfileForm } from "./profileForm"

/** The fields of a model profile, with the errors a save or test reported for them. */
export function ModelProfileFields({
  idPrefix,
  form,
  onChange,
  error,
}: {
  idPrefix: string
  form: ProfileForm
  onChange: (form: ProfileForm) => void
  error: unknown
}) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const id = (name: string) => `${idPrefix}-${name}`
  const modelIdError = errorForField(error, "model_id")
  const windowError = errorForField(error, "context_window")
  const outputError = errorForField(error, "max_output_tokens")

  return (
    <>
      <Field data-invalid={modelIdError !== undefined}>
        <FieldLabel htmlFor={id("model-id")}>{t(($) => $.settings.profile.modelId)}</FieldLabel>
        <Input
          id={id("model-id")}
          value={form.modelId}
          onChange={(event) => onChange({ ...form, modelId: event.target.value })}
          aria-invalid={modelIdError !== undefined}
          autoComplete="off"
          spellCheck={false}
        />
        {modelIdError !== undefined && <FieldError>{errorText(modelIdError)}</FieldError>}
      </Field>
      <div className="grid gap-4 sm:grid-cols-2">
        <Field data-invalid={windowError !== undefined}>
          <FieldLabel htmlFor={id("context-window")}>
            {t(($) => $.settings.profile.contextWindow)}
          </FieldLabel>
          <Input
            id={id("context-window")}
            type="number"
            inputMode="numeric"
            min={1}
            step={1}
            value={form.contextWindow}
            onChange={(event) => onChange({ ...form, contextWindow: event.target.value })}
            aria-invalid={windowError !== undefined}
          />
          {windowError !== undefined && <FieldError>{errorText(windowError)}</FieldError>}
        </Field>
        <Field data-invalid={outputError !== undefined}>
          <FieldLabel htmlFor={id("max-output")}>
            {t(($) => $.settings.profile.maxOutputTokens)}
          </FieldLabel>
          <Input
            id={id("max-output")}
            type="number"
            inputMode="numeric"
            min={1}
            step={1}
            value={form.maxOutputTokens}
            onChange={(event) => onChange({ ...form, maxOutputTokens: event.target.value })}
            aria-invalid={outputError !== undefined}
          />
          {outputError !== undefined && <FieldError>{errorText(outputError)}</FieldError>}
        </Field>
      </div>
      <Field orientation="horizontal">
        <Switch
          id={id("thinking")}
          checked={form.supportsThinking}
          onCheckedChange={(checked) => onChange({ ...form, supportsThinking: checked })}
        />
        <FieldContent>
          <FieldLabel htmlFor={id("thinking")}>
            {t(($) => $.settings.profile.supportsThinking)}
          </FieldLabel>
          <FieldDescription>{t(($) => $.settings.profile.supportsThinkingHint)}</FieldDescription>
        </FieldContent>
      </Field>
      <Field orientation="horizontal">
        <Switch
          id={id("usage")}
          checked={form.reportsStreamUsage}
          onCheckedChange={(checked) => onChange({ ...form, reportsStreamUsage: checked })}
        />
        <FieldContent>
          <FieldLabel htmlFor={id("usage")}>
            {t(($) => $.settings.profile.reportsStreamUsage)}
          </FieldLabel>
          <FieldDescription>{t(($) => $.settings.profile.reportsStreamUsageHint)}</FieldDescription>
        </FieldContent>
      </Field>
    </>
  )
}
