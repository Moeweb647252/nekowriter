import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useSearchParams } from "react-router"

import { AppFrame } from "@/components/AppFrame"
import { ModelSettingsPanel } from "@/components/ModelSettingsPanel"
import { ProjectSettingsPanel } from "@/components/ProjectSettingsPanel"
import { PromptSettingsPanel } from "@/components/PromptSettingsPanel"
import { ServicePanel } from "@/components/ServicePanel"
import { getHealth } from "@/lib/api"
import { useEventSync } from "@/lib/event-sync-context"

export function SettingsPage() {
  const [searchParams] = useSearchParams()
  const queryClient = useQueryClient()
  const { markServiceStopped, serviceStopped, settingsVersion } = useEventSync()
  const projectId = searchParams.get("project")
  const health = useQuery({
    queryKey: ["health"],
    queryFn: ({ signal }) => getHealth(signal),
  })

  return (
    <AppFrame
      title="系统设置"
      description={
        projectId
          ? "正在查看当前作品的模型与提示词覆盖。"
          : "配置本地写作服务和全局内置提示词。"
      }
    >
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 px-4 py-6 sm:px-6 lg:px-8">
        <ModelSettingsPanel
          refreshToken={settingsVersion}
          onModelOperation={() => {
            void queryClient.invalidateQueries({ queryKey: ["dashboard"] })
          }}
        />
        <ProjectSettingsPanel
          projectId={projectId}
          refreshToken={settingsVersion}
          onModelOperation={() => {
            void queryClient.invalidateQueries({ queryKey: ["dashboard"] })
          }}
        />
        <PromptSettingsPanel
          projectId={projectId}
          refreshToken={settingsVersion}
        />
        <ServicePanel
          health={health.data ?? null}
          stopped={serviceStopped}
          onStopped={markServiceStopped}
        />
      </div>
    </AppFrame>
  )
}
