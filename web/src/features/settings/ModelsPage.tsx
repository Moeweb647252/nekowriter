/** Models section (PROJECT §8.12): model profiles with Fetch models, the context window, the
 * output cap and the protocol options (spec/10 §2.1, spec/04 §5). */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Cpu, Pencil, Plus, RefreshCw, Trash2, TriangleAlert } from "lucide-react"
import { toast } from "sonner"

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
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
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
  useOptionSpecs,
  usePatchModelProfile,
  useProviders,
  useRemoteModels,
} from "@/api/hooks/providers"
import {
  OPTION_KEYS,
  clearUnreachableOptions,
  optionHintKey,
  optionLabelKey,
  optionValueKey,
  optionsForProtocol,
  providerProtocol,
  removeOptionKeys,
  unreachableOptionKeys,
  type OptionKey,
} from "@/features/settings/modelOptions"
import type { ModelOptionSpecDto } from "@/api/gen/ModelOptionSpecDto"
import type { Protocol } from "@/api/gen/Protocol"
import type { ProviderDto } from "@/api/gen/ProviderDto"
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

/** A draft option value as the form holds it: every field of `ModelOptionsDto` is optional and
 * nullable, and `extra_body` is a JSON object. */
type OptionValue = ModelOptionsDto[OptionKey]

export default function ModelsPage() {
  const { t } = useTranslation()
  const models = useModelProfiles()
  const providers = useProviders()
  const specs = useOptionSpecs()
  const remove = useDeleteModelProfile()
  const patchModel = usePatchModelProfile()
  const showError = useErrorToast()
  const [creating, setCreating] = useState(false)
  const [clearing, setClearing] = useState<ModelProfileDto | null>(null)
  const [editing, setEditing] = useState<ModelProfileDto | null>(null)
  const [deleting, setDeleting] = useState<ModelProfileDto | null>(null)

  if (models.isPending || providers.isPending || specs.isPending) {
    return <Skeleton className="h-64" />
  }
  if (models.isError || providers.isError || specs.isError) {
    return <ErrorToastInline />
  }
  const list = models.data ?? []
  const providerList = providers.data ?? []
  const specList = specs.data ?? []
  const providerName = (id: number) =>
    providerList.find((provider) => provider.id === id)?.name ?? String(id)
  const optionList = (keys: readonly string[]) =>
    keys
      .map((key) => t(optionLabelKey(key), { defaultValue: key }))
      .join(", ")

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
              <TableHead>{t("settings:protocol_options")}</TableHead>
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
                {/* Grouping only: both supported locales format these the same, so this
                    matches `formatNumber`; revisit when a locale is added (spec/11 §6). */}
                <TableCell>{model.context_window.toLocaleString()}</TableCell>
                <TableCell>{model.max_output_tokens.toLocaleString()}</TableCell>
                <TableCell>
                  {model.ignored_options.length === 0 ? (
                    <span className="text-xs text-muted-foreground">
                      {t("settings:option_count", { count: optionCount(specList, model, providerList) })}
                    </span>
                  ) : (
                    // A stored mismatch written before this rule existed. The badge is the fix
                    // entry point: nothing about it is silent any more (D-132).
                    <button
                      type="button"
                      data-testid={`ignored-options-${model.id}`}
                      onClick={() => setClearing(model)}
                      className="text-left"
                    >
                      <Badge
                        variant="destructive"
                        className="h-auto max-w-64 items-start gap-1 py-1 text-left whitespace-normal"
                      >
                        <TriangleAlert className="mt-0.5 size-3 shrink-0" />
                        {t("settings:option_mismatch_badge", {
                          options: optionList(model.ignored_options),
                        })}
                      </Badge>
                    </button>
                  )}
                </TableCell>
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
          specs={specList}
          providers={providerChoices(providerList)}
          onClose={() => setCreating(false)}
        />
      ) : null}
      {editing === null ? null : (
        <ModelDialog
          model={editing}
          specs={specList}
          providers={providerChoices(providerList)}
          onClose={() => setEditing(null)}
        />
      )}
      <ConfirmDialog
        open={clearing !== null}
        onOpenChange={() => setClearing(null)}
        title={t("settings:option_mismatch")}
        description={t("settings:option_mismatch_confirm", {
          options: optionList(clearing?.ignored_options ?? []),
          protocol: protocolLabel(t, providerProtocol(providerList, clearing?.provider_id ?? 0)),
        })}
        confirmLabel={t("settings:option_mismatch_fix")}
        onConfirm={() => {
          if (clearing === null) {
            return
          }
          // The author must be able to repair their own profile from the UI (D-132): the stored
          // options their protocol cannot send are removed, and nothing else changes.
          const input: ModelProfileInput = {
            provider_id: clearing.provider_id,
            display_name: clearing.display_name,
            model: clearing.model,
            context_window: clearing.context_window,
            max_output_tokens: clearing.max_output_tokens,
            options: removeOptionKeys(clearing.options, clearing.ignored_options),
          }
          patchModel.mutate(
            { id: clearing.id, input },
            {
              onSuccess: () => {
                toast.success(t("settings:option_mismatch_fixed"))
                setClearing(null)
              },
              onError: showError,
            },
          )
        }}
      />
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
  specs,
  providers,
  onClose,
}: {
  model?: ModelProfileDto
  specs: readonly ModelOptionSpecDto[]
  providers: readonly ProviderChoice[]
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
  // What a provider switch dropped, so the author sees it instead of finding it gone later.
  const [cleared, setCleared] = useState<OptionKey[]>([])
  const editing = model !== undefined
  const protocol = providerProtocol(providers, draft.provider_id)
  const visibleSpecs = optionsForProtocol(specs, protocol)
  // Live, not `model.ignored_options`: after a provider switch the stored list describes the old
  // protocol, and an alert that contradicts the fields on screen is worse than none.
  const stranded = unreachableOptionKeys(draft.options, specs, protocol)

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
            <div className="flex items-center gap-2">
              <Label htmlFor="model-provider">{t("settings:provider")}</Label>
              {protocol === null ? null : (
                <Badge variant="secondary" data-testid="model-protocol">
                  {protocolLabel(t, protocol)}
                </Badge>
              )}
            </div>
            <Select
              value={draft.provider_id}
              onValueChange={(value) => {
                // Switching to a provider with another protocol strands the options that protocol
                // cannot send. Dropping them here — visibly — is what keeps the original silent
                // no-op from being recreated on a protocol change (D-132).
                const nextProtocol = providerProtocol(providers, value)
                const result = clearUnreachableOptions(draft.options, specs, nextProtocol)
                setCleared(result.removed)
                setDraft({ ...draft, provider_id: value, options: result.options })
              }}
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
          {stranded.length === 0 ? null : (
            <Alert variant="destructive" data-testid="ignored-options-alert">
              <TriangleAlert />
              <AlertTitle>{t("settings:option_mismatch")}</AlertTitle>
              <AlertDescription>
                <p>
                  {t("settings:option_mismatch_body", {
                    options: stranded
                      .map((key) => t(optionLabelKey(key), { defaultValue: key }))
                      .join(", "),
                    protocol: protocolLabel(t, protocol),
                  })}
                </p>
                <Button
                  variant="outline"
                  size="sm"
                  className="mt-2"
                  onClick={() => {
                    setDraft({
                      ...draft,
                      options: removeOptionKeys(draft.options, stranded),
                    })
                    setCleared([])
                  }}
                >
                  {t("settings:option_mismatch_fix")}
                </Button>
              </AlertDescription>
            </Alert>
          )}
          {cleared.length === 0 ? null : (
            <Alert data-testid="cleared-options-alert">
              <TriangleAlert />
              <AlertTitle>{t("settings:option_cleared_title")}</AlertTitle>
              <AlertDescription>
                {t("settings:option_cleared_body", {
                  options: cleared
                    .map((key) => t(optionLabelKey(key), { defaultValue: key }))
                    .join(", "),
                  protocol: protocolLabel(t, protocol),
                })}
              </AlertDescription>
            </Alert>
          )}
          <div className="space-y-2">
            <div className="flex items-baseline justify-between">
              <Label>{t("settings:protocol_options")}</Label>
              <span className="text-xs text-muted-foreground">
                {protocol === null
                  ? null
                  : t("settings:option.protocol_note", {
                      protocol: protocolLabel(t, protocol),
                    })}
              </span>
            </div>
            {visibleSpecs.length === 0 ? (
              <p className="text-xs text-muted-foreground">
                {t("settings:option.none_for_protocol")}
              </p>
            ) : (
              <div className="space-y-2">
                {visibleSpecs.map((spec) => (
                  <OptionField
                    key={spec.key}
                    spec={spec}
                    value={draft.options[spec.key as OptionKey]}
                    onChange={(next) => {
                      const options: ModelOptionsDto = { ...draft.options }
                      if (next === undefined) {
                        delete options[spec.key as OptionKey]
                      } else {
                        assignOption(options, spec.key as OptionKey, next)
                      }
                      setDraft({ ...draft, options })
                    }}
                  />
                ))}
              </div>
            )}
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

/** One provider row as the form needs it: the protocol decides which options are offered. */
type ProviderChoice = { id: number; name: string; protocol: Protocol }

/** The provider list reduced to what the form uses. */
function providerChoices(providers: readonly ProviderDto[]): ProviderChoice[] {
  return providers.map((provider) => ({
    id: provider.id,
    name: provider.name,
    protocol: provider.protocol,
  }))
}

/** The localized name of a protocol, for the option notes and the mismatch text. */
function protocolLabel(
  t: (key: string, options?: { defaultValue?: string }) => string,
  protocol: Protocol | null,
): string {
  return protocol === null
    ? t("settings:protocol.mock")
    : t(`settings:protocol.${protocol}`, { defaultValue: protocol })
}

/** How many options a profile sets that its protocol can send (the "no mismatch" cell). */
function optionCount(
  specs: readonly ModelOptionSpecDto[],
  model: ModelProfileDto,
  providers: readonly ProviderDto[],
): number {
  const reachable = new Set(
    optionsForProtocol(specs, providerProtocol(providers, model.provider_id)).map(
      (spec) => spec.key,
    ),
  )
  return OPTION_KEYS.filter((key) => {
    const value = model.options[key]
    return value !== undefined && value !== null && reachable.has(key)
  }).length
}

/** Assigns one option value, narrowing to the field's own type so the DTO stays type-safe. */
function assignOption(options: ModelOptionsDto, key: OptionKey, value: OptionValue): void {
  switch (key) {
    case "reasoning_replay":
    case "reasoning_effort":
    case "max_tokens_field":
    case "effort":
    case "thinking_display":
    case "cache_ttl":
      options[key] = String(value)
      return
    case "send_parallel_tool_calls":
    case "stream_usage":
    case "refusal_fallback":
    case "eager_input_streaming":
      options[key] = Boolean(value)
      return
    case "temperature":
      options[key] = Number(value)
      return
    case "extra_body":
      // `JsonOptionField` parses before it reports, so the object is ready to store as-is.
      options.extra_body = value as ModelOptionsDto["extra_body"]
      return
  }
}

/** One option row: a label, a short explanation, and the widget its `kind` asks for.
 *
 * The widget is chosen by the server's option table, not by a list here, so adding an option to
 * the adapters shows up here without a second edit.
 */
function OptionField({
  spec,
  value,
  onChange,
}: {
  spec: ModelOptionSpecDto
  value: OptionValue
  onChange: (next: OptionValue) => void
}) {
  const { t } = useTranslation()
  const label = t(optionLabelKey(spec.key), { defaultValue: spec.key })
  const hint = t(optionHintKey(spec.key), { defaultValue: "" })
  const id = `model-option-${spec.key}`

  return (
    <div className="space-y-1">
      <Label htmlFor={id} className="text-xs">
        {label}
      </Label>
      {spec.kind === "choice" ? (
        <Select
          value={typeof value === "string" ? value : ""}
          onValueChange={(next) => onChange(next === "" ? undefined : next)}
        >
          <SelectTrigger id={id} aria-label={label}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="">
              {spec.default_value === null
                ? t("settings:option_unset")
                : t("settings:option_default_named", {
                    value: t(optionValueKey(spec.key, spec.default_value), {
                      defaultValue: spec.default_value,
                    }),
                  })}
            </SelectItem>
            {spec.values.map((option) => (
              <SelectItem key={option} value={option}>
                {t(optionValueKey(spec.key, option), { defaultValue: option })}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ) : spec.kind === "boolean" ? (
        <Select
          value={value === true ? "true" : value === false ? "false" : ""}
          onValueChange={(next) => onChange(next === "" ? undefined : next === "true")}
        >
          <SelectTrigger id={id} aria-label={label}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="">{t("settings:option_unset")}</SelectItem>
            <SelectItem value="true">{t("settings:option_on")}</SelectItem>
            <SelectItem value="false">{t("settings:option_off")}</SelectItem>
          </SelectContent>
        </Select>
      ) : spec.kind === "number" ? (
        <Input
          id={id}
          inputMode="decimal"
          value={value === undefined || value === null ? "" : String(value)}
          onChange={(event) => {
            const text = event.target.value.trim()
            if (text === "") {
              onChange(undefined)
              return
            }
            const parsed = Number(text)
            if (Number.isFinite(parsed)) {
              onChange(parsed)
            }
          }}
        />
      ) : spec.kind === "text" ? (
        <Input
          id={id}
          value={value === undefined || value === null ? "" : String(value)}
          onChange={(event) =>
            onChange(event.target.value === "" ? undefined : event.target.value)
          }
        />
      ) : (
        <JsonOptionField id={id} label={label} value={value} onChange={onChange} />
      )}
      {hint === "" ? null : <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  )
}

/** The `json` widget: a textarea that parses before it reports, and shows why it cannot. */
function JsonOptionField({
  id,
  label,
  value,
  onChange,
}: {
  id: string
  label: string
  value: OptionValue
  onChange: (next: OptionValue) => void
}) {
  const { t } = useTranslation()
  const [text, setText] = useState(() =>
    value === undefined || value === null || typeof value !== "object"
      ? ""
      : JSON.stringify(value, null, 2),
  )
  const [error, setError] = useState<string | null>(null)
  return (
    <div className="space-y-1">
      <textarea
        id={id}
        aria-label={label}
        className="min-h-16 w-full rounded-md border bg-transparent px-2 py-1 font-mono text-xs"
        value={text}
        placeholder="{ }"
        onChange={(event) => {
          const next = event.target.value
          setText(next)
          if (next.trim() === "") {
            setError(null)
            onChange(undefined)
            return
          }
          try {
            const parsed: unknown = JSON.parse(next)
            if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
              setError(t("settings:option_json_object"))
              return
            }
            setError(null)
            onChange(parsed as OptionValue)
          } catch {
            setError(t("settings:option_json_invalid"))
          }
        }}
      />
      {error === null ? null : <p className="text-xs text-status-failed">{error}</p>}
    </div>
  )
}
