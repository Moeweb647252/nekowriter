import { AppFrame } from "@/components/AppFrame"
import { KnowledgePanel } from "@/components/KnowledgePanel"
import { useEventSync } from "@/lib/events"

export function KnowledgePage() {
  const { knowledgeVersion } = useEventSync()
  return (
    <AppFrame
      title="知识库"
      description="管理可供写作流程检索的本地资料。"
    >
      <div className="mx-auto w-full max-w-6xl px-4 py-6 sm:px-6 lg:px-8">
        <KnowledgePanel refreshToken={knowledgeVersion} />
      </div>
    </AppFrame>
  )
}
