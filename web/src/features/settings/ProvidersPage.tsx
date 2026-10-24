/** Providers section (PROJECT §8.12): the provider table and its form with a preset, Test and
 * a write-only API key (spec/10 §2.1, spec/04 §8). */
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { CheckCircle2, Pencil, Plus, Server, Trash2, XCircle } from "lucide-react"

import { useErrorToast } from "@/components/common/ErrorToast"
import { EmptyState } from "@/components/common/EmptyState"
import { ErrorToastInline } from "@/features/settings/ErrorToastInline"
import { ConfirmDialog } from "@/components/common/ConfirmDialog"
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
  useCreateProvider,
  useDeleteProvider,
  usePatchProvider,
  useProviderPresets,
  useProviders,
  useTestProvider,
} from "@/api/hooks/providers"
import type { Protocol } from "@/api/gen/Protocol"
import type { ProviderDto } from "@/api/gen/ProviderDto"
import type { ProviderInput } from "@/api/gen/ProviderInput"
import type { ProviderPresetDto } from "@/api/gen/ProviderPresetDto"
import type { ProviderTestDto } from "@/api/gen/ProviderTestDto"

/** The editable form of a provider. The key is write-only (spec/11 §5.5, PROJECT §8.12). */
type ProviderDraft = {
  name: string
  protocol: Protocol
  base_url: string
  api_key: string
  extra_headers: string
}

const EMPTY_DRAFT: ProviderDraft = {
  name: "",
  protocol: "openai",
  base_url: "",
  api_key: "",
  extra_headers: "",
}

export default function ProvidersPage() {
  const { t } = useTranslation()
  const providers = useProviders()
  const presets = useProviderPresets()
  const remove = useDeleteProvider()
  const showError = useErrorToast()
  const [editing, setEditing] = useState<ProviderDto | null>(null)
  const [creating, setCreating] = useState(false)
  const [deleting, setDeleting] = useState<ProviderDto | null>(null)

  if (providers.isPending) {
    return <Skeleton className="h-64" />
  }
  if (providers.isError) {
    return <ErrorToastInline />
  }
  const list = providers.data ?? []

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-base font-semibold">{t("settings:providers")}</h2>
          <p className="text-sm text-muted-foreground">{t("settings:providers_hint")}</p>
        </div>
        <Button size="sm" onClick={() => setCreating(true)}>
          <Plus className="size-4" />
          {t("settings:add_provider")}
        </Button>
      </div>

      {list.length === 0 ? (
        <EmptyState
          title={t("settings:no_providers")}
          description={t("settings:no_providers_hint")}
        />
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("settings:name")}</TableHead>
              <TableHead>{t("settings:protocol")}</TableHead>
              <TableHead>{t("settings:base_url")}</TableHead>
              <TableHead>{t("settings:api_key")}</TableHead>
              <TableHead className="w-0" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {list.map((provider) => (
              <TableRow key={provider.id}>
                <TableCell className="font-medium">{provider.name}</TableCell>
                <TableCell>
                  <Badge variant="secondary">{t(`settings:protocol.${provider.protocol}`)}</Badge>
                </TableCell>
                <TableCell className="max-w-80 truncate font-mono text-xs">
                  {provider.base_url}
                </TableCell>
                <TableCell className="font-mono text-xs">
                  {provider.has_api_key
                    ? (provider.api_key_masked ?? t("settings:key_set"))
                    : t("settings:no_key")}
                </TableCell>
                <TableCell>
                  <div className="flex justify-end gap-1">
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={t("common:edit")}
                      onClick={() => setEditing(provider)}
                    >
                      <Pencil className="size-3.5" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={t("common:delete")}
                      onClick={() => setDeleting(provider)}
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
        <ProviderDialog
          presets={presets.data ?? []}
          onClose={() => setCreating(false)}
        />
      ) : null}
      {editing === null ? null : (
        <ProviderDialog
          provider={editing}
          presets={presets.data ?? []}
          onClose={() => setEditing(null)}
        />
      )}
      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={() => setDeleting(null)}
        title={t("settings:delete_provider_title", { name: deleting?.name ?? "" })}
        description={t("settings:delete_provider_body")}
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

/** Creates or edits one provider, with the preset helper and the connection test. */
function ProviderDialog({
  provider,
  presets,
  onClose,
}: {
  provider?: ProviderDto
  presets: ProviderPresetDto[]
  onClose: () => void
}) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const create = useCreateProvider()
  const patch = usePatchProvider()
  const test = useTestProvider()
  const [draft, setDraft] = useState<ProviderDraft>(
    provider === undefined
      ? EMPTY_DRAFT
      : {
          name: provider.name,
          protocol: provider.protocol,
          base_url: provider.base_url,
          api_key: "",
          extra_headers: serializeHeaders(provider.extra_headers),
        },
  )
  const [presetKey, setPresetKey] = useState<string>("custom")
  const [testResult, setTestResult] = useState<ProviderTestDto | null>(null)
  const editing = provider !== undefined

  const applyPreset = (key: string) => {
    setPresetKey(key)
    const preset = presets.find((candidate) => candidate.key === key)
    if (preset === undefined) {
      return
    }
    setDraft((current) => ({
      ...current,
      name: current.name === "" ? preset.name : current.name,
      protocol: preset.protocol,
      base_url: preset.base_url,
    }))
  }

  const submit = () => {
    const headers = parseHeaders(draft.extra_headers)
    if (headers === null) {
      showError(new Error("invalid extra headers"))
      return
    }
    const input: ProviderInput = {
      name: draft.name.trim(),
      protocol: draft.protocol,
      base_url: draft.base_url.trim(),
      extra_headers: headers,
    }
    // An empty key means "unchanged" when editing and "no key" when creating.
    if (draft.api_key !== "") {
      input.api_key = draft.api_key
    }
    if (editing) {
      patch.mutate(
        { id: provider.id, input },
        { onSuccess: onClose, onError: showError },
      )
    } else {
      create.mutate(input, { onSuccess: onClose, onError: showError })
    }
  }

  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>
            <Server className="mr-2 inline size-4" />
            {editing ? t("settings:edit_provider") : t("settings:add_provider")}
          </DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          {editing ? null : (
            <div className="space-y-1.5">
              <Label htmlFor="preset">{t("settings:preset")}</Label>
              <Select value={presetKey} onValueChange={applyPreset}>
                <SelectTrigger id="preset">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {presets.map((preset) => (
                    <SelectItem key={preset.key} value={preset.key}>
                      {preset.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
          <div className="space-y-1.5">
            <Label htmlFor="provider-name">{t("settings:name")}</Label>
            <Input
              id="provider-name"
              value={draft.name}
              onChange={(event) => setDraft({ ...draft, name: event.target.value })}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="provider-protocol">{t("settings:protocol")}</Label>
            <Select
              value={draft.protocol}
              onValueChange={(value) =>
                setDraft({ ...draft, protocol: value as Protocol })
              }
            >
              <SelectTrigger id="provider-protocol">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="openai">{t("settings:protocol.openai")}</SelectItem>
                <SelectItem value="anthropic">{t("settings:protocol.anthropic")}</SelectItem>
                <SelectItem value="mock">{t("settings:protocol.mock")}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="provider-url">{t("settings:base_url")}</Label>
            <Input
              id="provider-url"
              value={draft.base_url}
              placeholder="https://api.openai.com/v1"
              onChange={(event) => setDraft({ ...draft, base_url: event.target.value })}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="provider-key">{t("settings:api_key")}</Label>
            <Input
              id="provider-key"
              type="password"
              value={draft.api_key}
              placeholder={
                editing
                  ? (provider.api_key_masked ?? t("settings:key_unchanged"))
                  : t("settings:key_placeholder")
              }
              onChange={(event) => setDraft({ ...draft, api_key: event.target.value })}
            />
            <p className="text-xs text-muted-foreground">{t("settings:api_key_hint")}</p>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="provider-headers">{t("settings:extra_headers")}</Label>
            <Input
              id="provider-headers"
              value={draft.extra_headers}
              placeholder={t("settings:extra_headers_placeholder")}
              onChange={(event) =>
                setDraft({ ...draft, extra_headers: event.target.value })
              }
            />
          </div>
          {testResult === null ? null : (
            <div className="rounded-md border p-2 text-xs">
              {testResult.ok ? (
                <span className="flex items-center gap-1.5 text-status-done">
                  <CheckCircle2 className="size-3.5" />
                  {t("settings:test_ok", { ms: testResult.latency_ms })}
                  {testResult.text === undefined || testResult.text === null ? null : (
                    <span className="text-muted-foreground">“{testResult.text}”</span>
                  )}
                </span>
              ) : (
                <span className="flex items-center gap-1.5 text-status-failed">
                  <XCircle className="size-3.5" />
                  {t(`errors:${testResult.error?.code ?? "provider.unavailable"}`, {
                    defaultValue: testResult.error?.message ?? "",
                  })}
                </span>
              )}
            </div>
          )}
        </div>
        <DialogFooter className="gap-2 sm:justify-between">
          <Button
            variant="outline"
            size="sm"
            disabled={!editing || test.isPending}
            onClick={() => {
              if (editing) {
                test.mutate(
                  { id: provider.id },
                  { onSuccess: setTestResult, onError: showError },
                )
              }
            }}
          >
            {t("settings:test_connection")}
          </Button>
          <div className="flex gap-2">
            <Button variant="ghost" size="sm" onClick={onClose}>
              {t("common:cancel")}
            </Button>
            <Button size="sm" onClick={submit} disabled={create.isPending || patch.isPending}>
              {t("common:save")}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/** Renders the extra headers map as `name: value` lines. */
function serializeHeaders(headers: Record<string, string>): string {
  return Object.entries(headers)
    .map(([name, value]) => `${name}: ${value}`)
    .join("\n")
}

/** Parses `name: value` lines; `null` means the text is malformed. */
export function parseHeaders(text: string): Record<string, string> | null {
  const headers: Record<string, string> = {}
  for (const line of text.split("\n")) {
    const trimmed = line.trim()
    if (trimmed === "") {
      continue
    }
    const index = trimmed.indexOf(":")
    if (index <= 0) {
      return null
    }
    headers[trimmed.slice(0, index).trim()] = trimmed.slice(index + 1).trim()
  }
  return headers
}
