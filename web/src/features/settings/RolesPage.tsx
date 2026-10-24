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
import type { AgentRole } from "@/api/gen/AgentRole"
import type { RoleModels } from "@/api/gen/RoleModels"

/** The five roles in the order the settings page lists them (spec/02 §3). */
const ROLES: AgentRole[] = ["conception", "researcher", "writer", "reviewer", "archivist"]

/** The role → i18n key suffix (spec/11 §6). */
const ROLE_LABEL_KEY: Record<AgentRole, string> = {
  conception: "settings:role.conception",
  researcher: "settings:role.researcher",
  writer: "settings:role.writer",
  reviewer: "settings:role.reviewer",
  archivist: "settings:role.archivist",
}

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
  const roles: RoleModels = settings.data.roles_defaults

  const labelFor = (modelId: number): string => {
    const model = modelList.find((candidate) => candidate.id === modelId)
    if (model === undefined) {
      return t("settings:model_missing")
    }
    const provider = providerList.find((candidate) => candidate.id === model.provider_id)
    return provider === undefined
      ? model.display_name
      : `${provider.name} · ${model.display_name}`
  }

  const currentRoles: RoleModels = {
    conception: roles.conception ?? null,
    researcher: roles.researcher ?? null,
    writer: roles.writer ?? null,
    reviewer: roles.reviewer ?? null,
    archivist: roles.archivist ?? null,
  }

  const setRole = (role: AgentRole, value: string) => {
    const next: RoleModels = {
      ...currentRoles,
      [role]: value === "" ? null : Number(value),
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
          {ROLES.map((role) => {
            const current = currentRoles[role]
            return (
              <div key={role} className="flex items-center gap-3">
                <Label htmlFor={`role-${role}`} className="w-32 shrink-0">
                  <UsersRound className="mr-1.5 inline size-3.5" />
                  {t(ROLE_LABEL_KEY[role])}
                </Label>
                <Select
                  value={current === undefined || current === null ? "" : String(current)}
                  onValueChange={(value) => setRole(role, value)}
                >
                  <SelectTrigger id={`role-${role}`} className="flex-1">
                    <SelectValue placeholder={t("settings:role_unset")} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="">{t("settings:role_unset")}</SelectItem>
                    {modelList.map((model) => (
                      <SelectItem key={model.id} value={String(model.id)}>
                        {labelFor(model.id)}
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
