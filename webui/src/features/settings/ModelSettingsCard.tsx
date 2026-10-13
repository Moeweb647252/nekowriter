import { type SubmitEvent, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { SystemSettingsView } from "@/api/generated/SystemSettingsView"
import type { TestModelRequest } from "@/api/generated/TestModelRequest"
import { type ApiKeyChange, useSaveSystemSettings, useSystemSettings } from "@/api/queries/settings"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
  FieldSet,
  FieldLegend,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { Switch } from "@/components/ui/switch"
import { ModelProfileFields } from "@/features/models/ModelProfileFields"
import { profileForm, profileFromForm } from "@/features/models/profileForm"
import { ModelTest } from "@/features/models/ModelTest"
import { useErrorText } from "@/i18n/errors"

/** The model service and the default model (§13.3). The API key is never shown. */
export function ModelSettingsCard() {
  const { t } = useTranslation()
  const settings = useSystemSettings()

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t(($) => $.settings.model.title)}</CardTitle>
        <CardDescription>{t(($) => $.settings.model.description)}</CardDescription>
      </CardHeader>
      <CardContent>
        {settings.isPending && <Skeleton className="h-40 w-full" />}
        {settings.isError && (
          <ErrorAlert error={settings.error} onRetry={() => void settings.refetch()} />
        )}
        {/* A new revision, from here or elsewhere, starts the form from the saved values. */}
        {settings.isSuccess && (
          <ModelSettingsForm key={settings.data.revision} settings={settings.data} />
        )}
      </CardContent>
    </Card>
  )
}

const FIELDS = [
  "api_base_url",
  "api_key",
  "model_id",
  "context_window",
  "max_output_tokens",
] as const

function ModelSettingsForm({ settings }: { settings: SystemSettingsView }) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const save = useSaveSystemSettings()
  const [apiBaseUrl, setApiBaseUrl] = useState(settings.apiBaseUrl ?? "")
  const [newKey, setNewKey] = useState("")
  const [clearKey, setClearKey] = useState(false)
  const [hasDefault, setHasDefault] = useState(settings.defaultModel !== null)
  const [profile, setProfile] = useState(() => profileForm(settings.defaultModel))

  const apiKey: ApiKeyChange =
    newKey !== ""
      ? { kind: "replace", key: newKey }
      : clearKey
        ? { kind: "clear" }
        : { kind: "keep" }

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    save.mutate(
      {
        settings,
        apiBaseUrl: apiBaseUrl.trim() === "" ? null : apiBaseUrl,
        apiKey,
        defaultModel: hasDefault ? profileFromForm(profile) : null,
      },
      { onSuccess: () => toast.success(t(($) => $.settings.model.saved)) },
    )
  }

  const testRequest = (): TestModelRequest => ({
    apiBaseUrl,
    ...(apiKey.kind === "keep" ? {} : { apiKey: apiKey.kind === "clear" ? null : apiKey.key }),
    model: profileFromForm(profile),
  })

  const urlError = errorForField(save.error, "api_base_url")
  const keyError = errorForField(save.error, "api_key")
  const formError = errorForForm(save.error, FIELDS)

  return (
    <form onSubmit={submit} className="flex flex-col gap-6">
      <FieldGroup>
        <Field data-invalid={urlError !== undefined}>
          <FieldLabel htmlFor="model-base-url">{t(($) => $.settings.model.baseUrl)}</FieldLabel>
          <Input
            id="model-base-url"
            type="url"
            value={apiBaseUrl}
            onChange={(event) => setApiBaseUrl(event.target.value)}
            aria-invalid={urlError !== undefined}
            placeholder="https://api.openai.com/v1"
            autoComplete="off"
            spellCheck={false}
          />
          <FieldDescription>{t(($) => $.settings.model.baseUrlHint)}</FieldDescription>
          {urlError !== undefined && <FieldError>{errorText(urlError)}</FieldError>}
        </Field>
        <Field data-invalid={keyError !== undefined}>
          <FieldLabel htmlFor="model-api-key">{t(($) => $.settings.model.apiKey)}</FieldLabel>
          <Input
            id="model-api-key"
            type="password"
            value={newKey}
            onChange={(event) => setNewKey(event.target.value)}
            aria-invalid={keyError !== undefined}
            autoComplete="new-password"
          />
          <FieldDescription>
            {clearKey && newKey === ""
              ? t(($) => $.settings.model.keyWillClear)
              : settings.apiKeySet
                ? t(($) => $.settings.model.apiKeyStored)
                : t(($) => $.settings.model.apiKeyNone)}
          </FieldDescription>
          {settings.apiKeySet && (
            <div>
              <Button
                type="button"
                variant="link"
                className="h-auto p-0"
                onClick={() => setClearKey(!clearKey)}
              >
                {clearKey
                  ? t(($) => $.settings.model.keepKey)
                  : t(($) => $.settings.model.clearKey)}
              </Button>
            </div>
          )}
          {keyError !== undefined && <FieldError>{errorText(keyError)}</FieldError>}
        </Field>
        <FieldSet>
          <FieldLegend>{t(($) => $.settings.model.defaultModel)}</FieldLegend>
          <Field orientation="horizontal">
            <Switch id="model-has-default" checked={hasDefault} onCheckedChange={setHasDefault} />
            <FieldContent>
              <FieldLabel htmlFor="model-has-default">
                {t(($) => $.settings.model.useDefaultModel)}
              </FieldLabel>
              {!hasDefault && (
                <FieldDescription>{t(($) => $.settings.model.noDefaultModel)}</FieldDescription>
              )}
            </FieldContent>
          </Field>
          {hasDefault && (
            <ModelProfileFields
              idPrefix="default-model"
              form={profile}
              onChange={setProfile}
              error={save.error}
            />
          )}
        </FieldSet>
      </FieldGroup>
      {formError !== undefined && <ErrorAlert error={formError} />}
      <div className="flex flex-wrap items-start gap-3">
        <Button type="submit" disabled={save.isPending}>
          {save.isPending ? t(($) => $.common.status.saving) : t(($) => $.settings.model.save)}
        </Button>
        <ModelTest request={testRequest} disabled={!hasDefault} />
      </div>
    </form>
  )
}
