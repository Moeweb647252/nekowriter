/** Models section (PROJECT §8.12): model profiles with Fetch models, the context window, the
 * output cap and the protocol options (spec/10 §2.1, spec/04 §5). */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Cpu, Pencil, Plus, RefreshCw, Trash2 } from "lucide-react"

import { ConfirmDialog } from "@/components/common/ConfirmDialog"
import { EmptyState } from "@/components/common/EmptyState"
import { useErrorToast } from "@/components/common/ErrorToast"
import { ErrorToastInline } from "@/features/settings/ErrorToastInline"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Skeleton } from "@/components/ui/skeleton"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import {
  useCreateModelProfile,
  useDeleteModelProfile,
  useModelProfiles,
  usePatchModelProfile,
  useProviders,
  useRemoteModels,
} from "@/api/hooks/providers"
import type { ModelOptionsDto } from "@/api/gen/ModelOptionsDto"
import type { ModelProfileDto } from "@/api/gen/ModelProfileDto"
import type { ModelProfileInput } from "@/api/gen/ModelProfileInput"
import type { RemoteModelDto } from "@/api/gen/RemoteModelDto"

/** The editable form of one model profile. Options the UI edits directly are lifted out. */
type ModelDraft = {
  provider_id: string
  display_name: string
  model: string
  context_window: string
  max_output_tokens: string
  options: ModelOptionsDto
}

const EMPTY_DRAFT: ModelDraft = {
  provider_id: "",
  display_name: "",
  model: "",
  context_window: "128000",
  max_output_tokens: "16384",
  options: {},
}

/** The option fields the settings page exposes (spec/04 §5). */
const OPTION_FIELDS = [
  { key: "reasoning_replay", label: "reasoning_replay", values: ["", "never", "always"] },
  {
    key: "max_tokens_field",
    label: "max_tokens_field",
    values: ["", "max_tokens", "max_completion_tokens"],
  },
  {
    key: "effort",
    label: "effort",
    values: ["", "low", "medium", "high", "xhigh", "max"],
  },
  {
    key: "thinking_display",
    label: "thinking_display",
    values: ["", "summarized", "omitted"],
  },
  { key: "cache_ttl", label: "cache_ttl", values: ["", "auto", "5m", "1h"] },
] as const satisfies ReadonlyArray<{
  key: keyof ModelOptionsDto
  label: string
  values: readonly string[]
}>

export default function ModelsPage() {
  const { t } = useTranslation()
  const models = useModelProfiles()
  const providers = useProviders()
  const remove = useDeleteModelProfile()
  const showError = useErrorToast()
  const [creating, setCreating] = useState(false)
  const [editing, setEditing] = useState<ModelProfileDto | null>(null)
  const [deleting, setDeleting] = useState<ModelProfileDto | null>(null)

  if (models.isPending || providers.isPending) {
    return <Skeleton className="h-64" />
  }
  if (models.isError || providers.isError) {
    return <ErrorToastInline />
  }
  const list = models.data ?? []
  const providerList = providers.data ?? []
  const providerName = (id: number) =>
    providerList.find((provider) => provider.id === id)?.name ?? String(id)

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-base font-semibold">{t("settings:models")}</h2>
          <p className="text-sm text-muted-foreground">{t("settings:models_hint")}</p>
        </div>
        <Button
          size="sm"
          disabled={providerList.length === 0}
          onClick={() => setCreating(true)}
        >
          <Plus className="size-4" />
          {t("settings:add_model")}
        </Button>
      </div>

      {providerList.length === 0 ? (
        <EmptyState
          title={t("settings:no_providers")}
          description={t("settings:no_providers_hint")}
        />
      ) : list.length === 0 ? (
        <EmptyState title={t("settings:no_models")} description={t("settings:no_models_hint")} />
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("settings:display_name")}</TableHead>
              <TableHead>{t("settings:model_id")}</TableHead>
              <TableHead>{t("settings:provider")}</TableHead>
              <TableHead>{t("settings:context_window")}</TableHead>
              <TableHead>{t("settings:max_output")}</TableHead>
              <TableHead className="w-0" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {list.map((model) => (
              <TableRow key={model.id}>
                <TableCell className="font-medium">{model.display_name}</TableCell>
                <TableCell className="font-mono text-xs">{model.model}</TableCell>
                <TableCell>
                  <Badge variant="secondary">{providerName(model.provider_id)}</Badge>
                </TableCell>
                <TableCell>{model.context_window.toLocaleString()}</TableCell>
                <TableCell>{model.max_output_tokens.toLocaleString()}</TableCell>
                <TableCell>
                  <div className="flex justify-end gap-1">
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={t("common:edit")}
                      onClick={() => setEditing(model)}
                    >
                      <Pencil className="size-3.5" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={t("common:delete")}
                      onClick={() => setDeleting(model)}
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}

      {creating ? (
        <ModelDialog
          providers={providerList.map((provider) => ({
            id: provider.id,
            name: provider.name,
          }))}
          onClose={() => setCreating(false)}
        />
      ) : null}
      {editing === null ? null : (
        <ModelDialog
          model={editing}
          providers={providerList.map((provider) => ({
            id: provider.id,
            name: provider.name,
          }))}
          onClose={() => setEditing(null)}
        />
      )}
      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={() => setDeleting(null)}
        title={t("settings:delete_model_title", { name: deleting?.display_name ?? "" })}
        description={t("settings:delete_model_body")}
        confirmLabel={t("common:delete")}
        onConfirm={() => {
          if (deleting !== null) {
            remove.mutate(deleting.id, { onError: showError })
          }
          setDeleting(null)
        }}
      />
    </div>
  )
}

/** Creates or edits one model profile; Fetch models pre-fills id, window and output cap. */
function ModelDialog({
  model,
  providers,
  onClose,
}: {
  model?: ModelProfileDto
  providers: Array<{ id: number; name: string }>
  onClose: () => void
}) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const create = useCreateModelProfile()
  const patchModel = usePatchModelProfile()
  const fetchModels = useRemoteModels()
  const [draft, setDraft] = useState<ModelDraft>(
    model === undefined
      ? {
          ...EMPTY_DRAFT,
          provider_id: providers[0] === undefined ? "" : String(providers[0].id),
        }
      : {
          provider_id: String(model.provider_id),
          display_name: model.display_name,
          model: model.model,
          context_window: String(model.context_window),
          max_output_tokens: String(model.max_output_tokens),
          options: model.options,
        },
  )
  const [remote, setRemote] = useState<RemoteModelDto[]>([])
  const [fetchError, setFetchError] = useState<string | null>(null)
  const editing = model !== undefined

  const applyRemote = (remoteModel: RemoteModelDto) => {
    setDraft((current) => ({
      ...current,
      model: remoteModel.id,
      display_name:
        current.display_name === "" ? (remoteModel.display_name ?? remoteModel.id) : current.display_name,
      context_window:
        remoteModel.context_window === undefined || remoteModel.context_window === null
          ? current.context_window
          : String(remoteModel.context_window),
      max_output_tokens:
        remoteModel.max_output_tokens === undefined || remoteModel.max_output_tokens === null
          ? current.max_output_tokens
          : String(remoteModel.max_output_tokens),
    }))
  }

  const submit = () => {
    const providerId = Number(draft.provider_id)
    if (!Number.isFinite(providerId) || providerId <= 0) {
      showError(new Error("provider"))
      return
    }
    const input: ModelProfileInput = {
      provider_id: providerId,
      display_name: draft.display_name.trim(),
      model: draft.model.trim(),
      context_window: Number(draft.context_window) || 0,
      max_output_tokens: Number(draft.max_output_tokens) || 0,
      options: draft.options,
    }
    if (editing) {
      patchModel.mutate({ id: model.id, input }, { onSuccess: onClose, onError: showError })
    } else {
      create.mutate(input, { onSuccess: onClose, onError: showError })
    }
  }

  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>
            <Cpu className="mr-2 inline size-4" />
            {editing ? t("settings:edit_model") : t("settings:add_model")}
          </DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label htmlFor="model-provider">{t("settings:provider")}</Label>
            <Select
              value={draft.provider_id}
              onValueChange={(value) => setDraft({ ...draft, provider_id: value })}
            >
              <SelectTrigger id="model-provider">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {providers.map((provider) => (
                  <SelectItem key={provider.id} value={String(provider.id)}>
                    {provider.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <div className="flex items-end gap-2">
              <div className="flex-1 space-y-1.5">
                <Label htmlFor="model-id">{t("settings:model_id")}</Label>
                <Input
                  id="model-id"
                  value={draft.model}
                  onChange={(event) => setDraft({ ...draft, model: event.target.value })}
                />
              </div>
              <Button
                variant="outline"
                size="sm"
                disabled={fetchModels.isPending}
                onClick={() => {
                  fetchModels.mutate(Number(draft.provider_id), {
                    onSuccess: (found) => {
                      setFetchError(null)
                      setRemote(found)
                    },
                    onError: (error) => {
                      setRemote([])
                      setFetchError(
                        error instanceof Error ? error.message : t("settings:fetch_failed"),
                      )
                    },
                  })
                }}
              >
                <RefreshCw className="size-3.5" />
                {t("settings:fetch_models")}
              </Button>
            </div>
            {fetchError === null ? null : (
              <p className="text-xs text-status-failed">{t("settings:fetch_failed")}</p>
            )}
            {remote.length === 0 ? null : (
              <div className="max-h-40 overflow-y-auto rounded-md border">
                {remote.map((remoteModel) => (
                  <button
                    key={remoteModel.id}
                    type="button"
                    className="block w-full px-2 py-1 text-left font-mono text-xs hover:bg-accent"
                    onClick={() => applyRemote(remoteModel)}
                  >
                    {remoteModel.id}
                    {remoteModel.display_name === undefined ||
                    remoteModel.display_name === null
                      ? null
                      : ` — ${remoteModel.display_name}`}
                  </button>
                ))}
              </div>
            )}
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="model-name">{t("settings:display_name")}</Label>
            <Input
              id="model-name"
              value={draft.display_name}
              onChange={(event) => setDraft({ ...draft, display_name: event.target.value })}
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="model-context">{t("settings:context_window")}</Label>
              <Input
                id="model-context"
                inputMode="numeric"
                value={draft.context_window}
                onChange={(event) =>
                  setDraft({ ...draft, context_window: event.target.value })
                }
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="model-output">{t("settings:max_output")}</Label>
              <Input
                id="model-output"
                inputMode="numeric"
                value={draft.max_output_tokens}
                onChange={(event) =>
                  setDraft({ ...draft, max_output_tokens: event.target.value })
                }
              />
            </div>
          </div>
          <div className="space-y-2">
            <Label>{t("settings:protocol_options")}</Label>
            <div className="grid grid-cols-2 gap-2">
              {OPTION_FIELDS.map((field) => {
                const value = draft.options[field.key]
                return (
                  <div key={field.key} className="space-y-1">
                    <span className="font-mono text-[0.7rem] text-muted-foreground">
                      {field.label}
                    </span>
                    <Select
                      value={typeof value === "string" ? value : ""}
                      onValueChange={(next) =>
                        setDraft({
                          ...draft,
                          options: {
                            ...draft.options,
                            [field.key]: next === "" ? undefined : next,
                          },
                        })
                      }
                    >
                      <SelectTrigger aria-label={field.label}>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {field.values.map((option) => (
                          <SelectItem key={option} value={option}>
                            {option === "" ? t("settings:option_default") : option}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                )
              })}
            </div>
          </div>
        </div>
        <DialogFooter>
          <Button variant="ghost" size="sm" onClick={onClose}>
            {t("common:cancel")}
          </Button>
          <Button size="sm" onClick={submit} disabled={create.isPending || patchModel.isPending}>
            {t("common:save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
