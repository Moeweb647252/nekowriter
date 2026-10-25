/** Settings › Embeddings (PROJECT §8.10, spec/07 §4): the model, its loading state and Rebuild. */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { RefreshCw } from "lucide-react"

import { useAppSettings, usePatchAppSettings } from "@/api/hooks/app-settings"
import { useEmbeddingStatus, useRebuildIndex } from "@/api/hooks/knowledge"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Skeleton } from "@/components/ui/skeleton"

/** The supported models of `crates/embed/models.toml` (spec/07 §4). */
const MODELS: { key: string; dimension: number }[] = [
  { key: "multilingual-e5-small", dimension: 384 },
  { key: "bge-small-zh-v1.5", dimension: 512 },
  { key: "embeddinggemma-300m-q", dimension: 768 },
]

export default function EmbeddingsPage() {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const settings = useAppSettings()
  const patch = usePatchAppSettings()
  const status = useEmbeddingStatus()
  const rebuild = useRebuildIndex()
  // The select shows the saved model until the author picks another one (derived state).
  const [modelChoice, setModelChoice] = useState<string | null>(null)

  if (settings.isPending || status.isPending || settings.data === undefined) {
    return <Skeleton className="h-64" />
  }

  const model = modelChoice ?? settings.data.embedding.model
  const dirty = model !== settings.data.embedding.model
  const state = status.data?.state ?? "not_loaded"

  return (
    <div className="max-w-2xl space-y-6">
      <div>
        <h2 className="text-base font-semibold">{t("settings:embeddings")}</h2>
        <p className="text-sm text-muted-foreground">{t("settings:embeddings_hint")}</p>
      </div>

      <div className="flex items-center gap-3">
        <Label htmlFor="embedding-model" className="w-40 shrink-0">
          {t("settings:embedding_model")}
        </Label>
        <Select value={model} onValueChange={setModelChoice}>
          <SelectTrigger id="embedding-model" className="flex-1">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {MODELS.map((entry) => (
              <SelectItem key={entry.key} value={entry.key}>
                {entry.key} ({entry.dimension})
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button
          disabled={!dirty || patch.isPending}
          onClick={() => patch.mutate({ embedding: { model } }, { onError: showError })}
        >
          {t("common:save")}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">{t("settings:embedding_model_hint")}</p>

      <div className="rounded-md border p-4">
        <div className="flex items-center gap-3">
          <span className="text-sm font-medium">{t("settings:index_status")}</span>
          <Badge variant={state === "failed" ? "destructive" : "secondary"}>
            {t(`settings:embedding_state.${state}`)}
          </Badge>
          <span className="font-mono text-xs text-muted-foreground">
            {status.data?.model}
          </span>
        </div>
        {status.data?.message === null ? null : (
          <p className="mt-2 text-xs text-destructive">{status.data?.message}</p>
        )}
        <div className="mt-3 flex gap-4 text-xs text-muted-foreground">
          <span>
            {t("settings:index_pending")}: {status.data?.pending ?? 0}
          </span>
          <span>
            {t("settings:index_done")}: {status.data?.done ?? 0}
          </span>
          <span>
            {t("settings:index_failed")}: {status.data?.failed ?? 0}
          </span>
        </div>
        <div className="mt-3 flex items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={rebuild.isPending}
            onClick={() => rebuild.mutate(undefined, { onError: showError })}
          >
            <RefreshCw className="size-3.5" /> {t("settings:rebuild_index")}
          </Button>
          <span className="text-xs text-muted-foreground">{t("settings:rebuild_hint")}</span>
        </div>
      </div>
    </div>
  )
}
