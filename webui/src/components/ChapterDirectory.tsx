import { Badge } from "@/components/ui/badge"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import type { ChapterSummary } from "@/lib/api"

interface ChapterDirectoryProps {
  chapters: ChapterSummary[]
  /** 当前正在写作的章节；它与被浏览的章节是两件事。 */
  activeChapterId: string | null
  /** 用户正在查看的章节；为空表示停留在写作区。 */
  browsedChapterId: string | null
  onSelect: (chapterId: string) => void
}

const statusLabels: Record<ChapterSummary["status"], string> = {
  planned: "未开始",
  in_progress: "撰写中",
  approved: "已通过",
}

function chapterLabel(chapter: ChapterSummary) {
  return chapter.kind === "extra"
    ? `番外 ${chapter.orderIndex}`
    : `第 ${chapter.orderIndex} 章`
}

export function ChapterDirectory({
  chapters,
  activeChapterId,
  browsedChapterId,
  onSelect,
}: ChapterDirectoryProps) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>章节目录</CardTitle>
        <CardDescription>
          点击任意章节查看已提交内容；查看不会改动写作进度。
        </CardDescription>
      </CardHeader>
      <CardContent>
        {chapters.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            粗纲审核通过后会在这里按顺序列出章节。
          </p>
        ) : (
          <ol className="space-y-1">
            {chapters.map((chapter) => {
              const isWriting = chapter.id === activeChapterId
              const isBrowsed = chapter.id === browsedChapterId
              return (
                <li key={chapter.id}>
                  <button
                    type="button"
                    aria-current={isBrowsed ? "true" : undefined}
                    onClick={() => onSelect(chapter.id)}
                    className={`flex w-full items-center justify-between gap-2 rounded-xl border px-2.5 py-2 text-left text-sm transition focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring ${
                      isBrowsed
                        ? "border-primary/50 bg-primary/10"
                        : "border-transparent hover:border-border hover:bg-muted/40"
                    }`}
                  >
                    <span className="flex min-w-0 flex-col">
                      <span className="truncate font-medium">
                        {chapterLabel(chapter)}
                      </span>
                      <span className="truncate text-xs text-muted-foreground">
                        {chapter.title}
                      </span>
                    </span>
                    <span className="flex shrink-0 flex-col items-end gap-1">
                      {isWriting ? (
                        <Badge variant="default">写作中</Badge>
                      ) : null}
                      <Badge variant="outline">
                        {statusLabels[chapter.status]}
                      </Badge>
                    </span>
                  </button>
                </li>
              )
            })}
          </ol>
        )}
      </CardContent>
    </Card>
  )
}
