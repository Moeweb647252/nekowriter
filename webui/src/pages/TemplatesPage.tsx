import { AppFrame } from "@/components/AppFrame"
import { TemplatePanel } from "@/components/TemplatePanel"
import { useEventSync } from "@/lib/events"

export function TemplatesPage() {
  const { templateVersion } = useEventSync()
  return (
    <AppFrame
      title="提示词模板"
      description="保存可插入到创作指导和反馈中的复用片段。"
    >
      <div className="mx-auto w-full max-w-6xl px-4 py-6 sm:px-6 lg:px-8">
        <TemplatePanel refreshToken={templateVersion} />
      </div>
    </AppFrame>
  )
}
