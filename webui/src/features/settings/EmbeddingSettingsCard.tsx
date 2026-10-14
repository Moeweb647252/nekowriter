import { CircleCheckIcon } from "lucide-react"
import { type SubmitEvent, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import { ApiKeyField } from "./ApiKeyField"
import { errorForField, errorForForm } from "@/api/fieldErrors"
import type { EmbeddingSettingsView } from "@/api/generated/EmbeddingSettingsView"
import {
  type ApiKeyChange,
  apiKeyField,
  useEmbeddingSettings,
  useEmbeddingTest,
  useSaveEmbeddingSettings,
} from "@/api/queries/settings"
import { ErrorAlert } from "@/components/app/ErrorAlert"
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { Switch } from "@/components/ui/switch"
import { useErrorText } from "@/i18n/errors"
import { useFormatters } from "@/i18n/format"

/** The embedding service and model for semantic knowledge search (§12.2, D10). */
export function EmbeddingSettingsCard() {
  const { t } = useTranslation()
  const settings = useEmbeddingSettings()
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t(($) => $.settings.embedding.title)}</CardTitle>
        <CardDescription>{t(($) => $.settings.embedding.description)}</CardDescription>
      </CardHeader>
      <CardContent>
        {settings.isPending && <Skeleton className="h-40 w-full" />}
        {settings.isError && (
          <ErrorAlert error={settings.error} onRetry={() => void settings.refetch()} />
        )}
        {/* A new revision, from here or elsewhere, starts the form from the saved values. */}
        {settings.isSuccess && (
          <EmbeddingSettingsForm key={settings.data.revision} settings={settings.data} />
        )}
      </CardContent>
    </Card>
  )
}

const FIELDS = ["api_base_url", "api_key", "model_id", "dimensions"] as const

function EmbeddingSettingsForm({ settings }: { settings: EmbeddingSettingsView }) {
  const { t } = useTranslation()
  const errorText = useErrorText()
  const format = useFormatters()
  const save = useSaveEmbeddingSettings()
  const test = useEmbeddingTest()
  const [apiBaseUrl, setApiBaseUrl] = useState(settings.apiBaseUrl ?? "")
  const [newKey, setNewKey] = useState("")
  const [clearKey, setClearKey] = useState(false)
  const [hasModel, setHasModel] = useState(settings.model !== null)
  const [modelId, setModelId] = useState(settings.model?.modelId ?? "")
  const [dimensions, setDimensions] = useState(String(settings.model?.dimensions ?? 1024))

  const apiKey: ApiKeyChange =
    newKey !== ""
      ? { kind: "replace", key: newKey }
      : clearKey
        ? { kind: "clear" }
        : { kind: "keep" }
  const model = { modelId, dimensions: Number(dimensions) }

  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    save.mutate(
      {
        settings,
        apiBaseUrl: apiBaseUrl.trim() === "" ? null : apiBaseUrl,
        apiKey,
        model: hasModel ? model : null,
      },
      { onSuccess: () => toast.success(t(($) => $.settings.embedding.saved)) },
    )
  }
  const runTest = () => {
    test.mutate({ apiBaseUrl, ...apiKeyField(apiKey), model })
  }

  const error = save.error ?? test.error
  const urlError = errorForField(error, "api_base_url")
  const keyError = errorForField(error, "api_key")
  const modelError = errorForField(error, "model_id")
  const dimensionsError = errorForField(error, "dimensions")
  const formError = errorForForm(save.error, FIELDS)
  const testError = errorForForm(test.error, FIELDS)
  return (
    <form onSubmit={submit} className="flex flex-col gap-6">
      <FieldGroup>
        <Field data-invalid={urlError !== undefined}>
          <FieldLabel htmlFor="embedding-base-url">{t(($) => $.settings.model.baseUrl)}</FieldLabel>
          <Input
            id="embedding-base-url"
            type="url"
            value={apiBaseUrl}
            onChange={(event) => setApiBaseUrl(event.target.value)}
            aria-invalid={urlError !== undefined}
            placeholder="https://api.openai.com/v1"
            autoComplete="off"
            spellCheck={false}
          />
          <FieldDescription>{t(($) => $.settings.embedding.baseUrlHint)}</FieldDescription>
          {urlError !== undefined && <FieldError>{errorText(urlError)}</FieldError>}
        </Field>
        <ApiKeyField
          id="embedding-api-key"
          stored={settings.apiKeySet}
          value={newKey}
          onChange={setNewKey}
          clear={clearKey}
          onClear={setClearKey}
          error={keyError}
        />
        <FieldSet>
          <FieldLegend>{t(($) => $.settings.embedding.model)}</FieldLegend>
          <Field orientation="horizontal">
            <Switch id="embedding-has-model" checked={hasModel} onCheckedChange={setHasModel} />
            <FieldContent>
              <FieldLabel htmlFor="embedding-has-model">
                {t(($) => $.settings.embedding.useModel)}
              </FieldLabel>
              {!hasModel && (
                <FieldDescription>{t(($) => $.settings.embedding.noModel)}</FieldDescription>
              )}
            </FieldContent>
          </Field>
          {hasModel && (
            <div className="grid gap-4 sm:grid-cols-2">
              <Field data-invalid={modelError !== undefined}>
                <FieldLabel htmlFor="embedding-model-id">
                  {t(($) => $.settings.profile.modelId)}
                </FieldLabel>
                <Input
                  id="embedding-model-id"
                  value={modelId}
                  onChange={(event) => setModelId(event.target.value)}
                  aria-invalid={modelError !== undefined}
                  spellCheck={false}
                />
                {modelError !== undefined && <FieldError>{errorText(modelError)}</FieldError>}
              </Field>
              <Field data-invalid={dimensionsError !== undefined}>
                <FieldLabel htmlFor="embedding-dimensions">
                  {t(($) => $.settings.embedding.dimensions)}
                </FieldLabel>
                <Input
                  id="embedding-dimensions"
                  type="number"
                  min={1}
                  value={dimensions}
                  onChange={(event) => setDimensions(event.target.value)}
                  aria-invalid={dimensionsError !== undefined}
                />
                <FieldDescription>{t(($) => $.settings.embedding.dimensionsHint)}</FieldDescription>
                {dimensionsError !== undefined && (
                  <FieldError>{errorText(dimensionsError)}</FieldError>
                )}
              </Field>
            </div>
          )}
        </FieldSet>
      </FieldGroup>
      {formError !== undefined && <ErrorAlert error={formError} />}
      <div className="flex flex-wrap items-start gap-3">
        <Button type="submit" disabled={save.isPending}>
          {save.isPending ? t(($) => $.common.status.saving) : t(($) => $.settings.embedding.save)}
        </Button>
        <div className="flex flex-col items-start gap-3">
          <Button
            type="button"
            variant="outline"
            disabled={!hasModel || test.isPending}
            onClick={runTest}
          >
            {test.isPending
              ? t(($) => $.settings.test.running)
              : t(($) => $.settings.embedding.test)}
          </Button>
          {testError !== undefined && <ErrorAlert error={testError} />}
          {test.isSuccess && (
            <Alert>
              <CircleCheckIcon />
              <AlertDescription>
                {t(($) => $.settings.embedding.passed, {
                  dimensions: test.data.dimensions,
                  seconds: format.number(Math.round(test.data.elapsedMillis / 100) / 10),
                })}
              </AlertDescription>
            </Alert>
          )}
        </div>
      </div>
    </form>
  )
}
