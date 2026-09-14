import { useState } from "react"
import { useQueryClient } from "@tanstack/react-query"

import { AppFrame } from "@/components/AppFrame"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { RecycleBinPanel } from "@/components/RecycleBinPanel"
import { useEventSync } from "@/lib/events"

export function RecyclePage() {
  const queryClient = useQueryClient()
  const [error, setError] = useState<string | null>(null)
  const { recycleVersion } = useEventSync()

  return (
    <AppFrame
      title="回收站"
      description="恢复项目、稿本和可安全清理的临时内容。"
    >
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 px-4 py-6 sm:px-6 lg:px-8">
        {error ? (
          <Alert variant="destructive">
            <AlertTitle>操作未完成</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        ) : null}
        <RecycleBinPanel
          refreshToken={recycleVersion}
          onChanged={() => {
            void queryClient.invalidateQueries({ queryKey: ["projects"] })
            void queryClient.invalidateQueries({ queryKey: ["dashboard"] })
          }}
          onError={setError}
        />
      </div>
    </AppFrame>
  )
}
