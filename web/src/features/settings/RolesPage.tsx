/** Agent roles section (PROJECT §8.12): one model profile per agent role (spec/10 §2.1
 * `roles.defaults`). */
import { useTranslation } from "react-i18next"
import { UsersRound } from "lucide-react"

import { useAppSettings, usePatchAppSettings } from "@/api/hooks/app-settings"
import { useModelProfiles, useProviders } from "@/api/hooks/providers"
import { useErrorToast } from "@/components/common/ErrorToast"
import { EmptyState } from "@/components/common/EmptyState"
import { ErrorToastInline } from "@/features/settings/ErrorToastInline"
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
import {
  AGENT_ROLES,
  formatModelLabel,
  roleLabelKey,
  roleSelection,
  type RoleSelection,
} from "@/features/settings/roleModels"
import { DEFAULT_OPTION_VALUE } from "@/features/settings/modelOptions"
import type { AgentRole } from "@/api/gen/AgentRole"
import type { RoleModels } from "@/api/gen/RoleModels"

/** The `Select` value of "no model assigned": the role falls back to the global default.
 *
 * A real value, not `""`: Radix's `SelectValue` treats an empty value as "no value" and renders
 * its placeholder, so "Not set" used to be grey hint text that disappeared rather than the
 * selected state it actually is (D-133).
 */
const ROLE_UNSET = DEFAULT_OPTION_VALUE

export default function RolesPage() {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const settings = useAppSettings()
  const models = useModelProfiles()
  const providers = useProviders()
  const patch = usePatchAppSettings()

  if (settings.isPending || models.isPending || providers.isPending) {
    return <Skeleton className="h-64" />
  }
  if (settings.isError || models.isError || providers.isError) {
    return <ErrorToastInline />
  }
  const modelList = models.data ?? []
  const providerList = providers.data ?? []
  // One entry per role (`null` = not set) and the shared provider-qualified model label, so this
  // page and the per-book overrides in Book settings cannot drift apart.
  const currentRoles: RoleSelection = roleSelection(settings.data.roles_defaults)

  const setRole = (role: AgentRole, value: string) => {
    const next: RoleModels = {
      ...currentRoles,
      [role]: value === ROLE_UNSET ? null : Number(value),
    }
    patch.mutate({ roles_defaults: next }, { onError: showError })
  }

  return (
    <div className="max-w-xl space-y-4">
      <div>
        <h2 className="text-base font-semibold">{t("settings:roles")}</h2>
        <p className="text-sm text-muted-foreground">{t("settings:roles_hint")}</p>
      </div>
      {modelList.length === 0 ? (
        <EmptyState title={t("settings:no_models")} description={t("settings:no_models_hint")} />
      ) : (
        <div className="space-y-3">
          {AGENT_ROLES.map((role) => {
            const current = currentRoles[role]
            return (
              <div key={role} className="flex items-center gap-3">
                <Label htmlFor={`role-${role}`} className="w-32 shrink-0">
                  <UsersRound className="mr-1.5 inline size-3.5" />
                  {t(roleLabelKey(role))}
                </Label>
                <Select
                  value={current === undefined || current === null ? ROLE_UNSET : String(current)}
                  onValueChange={(value) => setRole(role, value)}
                >
                  <SelectTrigger id={`role-${role}`} className="flex-1">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={ROLE_UNSET}>{t("settings:role_unset")}</SelectItem>
                    {modelList.map((model) => (
                      <SelectItem key={model.id} value={String(model.id)}>
                        {formatModelLabel(t, modelList, providerList, model.id)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )
          })}
          <Button
            variant="outline"
            size="sm"
            disabled={patch.isPending}
            onClick={() => {
              patch.mutate(
                {
                  roles_defaults: {
                    conception: null,
                    researcher: null,
                    writer: null,
                    reviewer: null,
                    archivist: null,
                  },
                },
                { onError: showError },
              )
            }}
          >
            {t("settings:roles_clear_all")}
          </Button>
        </div>
      )}
    </div>
  )
}
