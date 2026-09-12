import { useEffect, useState, type FormEvent } from "react"
import { LoaderCircle, Plus, Trash2 } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import {
  addExtraChapter,
  deleteExtraChapter,
  getChapters,
  type ChapterSummary,
  type PreparationSnapshot,
} from "@/lib/api"

type ExtraChapterPanelProps = {
  versionId: string
  preparation: PreparationSnapshot
  onChanged: (snapshot: PreparationSnapshot) => void
  onError: (message: string) => void
}

export function ExtraChapterPanel({
  versionId,
  preparation,
  onChanged,
  onError,
}: ExtraChapterPanelProps) {
  const [title, setTitle] = useState("")
  const [submitting, setSubmitting] = useState(false)
  const [extras, setExtras] = useState<ChapterSummary[]>([])
  const [deleting, setDeleting] = useState<string | null>(null)
  const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null)

  // 番外列表随服务端状态重新读取：新增与删除会改变工作流修订，而从回收站恢复
  // 一章不会——恢复只动数据与顺序。因此这里以服务端重读出来的快照本身作刷新信号，
  // 而不是它的某一个字段。
  useEffect(() => {
    let isCurrent = true
    getChapters(versionId)
      .then((chapters) => {
        if (isCurrent) {
          setExtras(chapters.filter((chapter) => chapter.kind === "extra"))
        }
      })
      .catch(() => {
        if (isCurrent) setExtras([])
      })
    return () => {
      isCurrent = false
    }
  }, [versionId, preparation])

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!title.trim()) return

    setSubmitting(true)
    try {
      onChanged(
        await addExtraChapter(
          versionId,
          preparation.workflow.stateRevision,
          title
        )
      )
      setTitle("")
    } catch (error) {
      onError(error instanceof Error ? error.message : "番外未能创建")
    } finally {
      setSubmitting(false)
    }
  }

  async function handleDelete(chapter: ChapterSummary) {
    if (confirmingDelete !== chapter.id) {
      setConfirmingDelete(chapter.id)
      return
    }
    setConfirmingDelete(null)
    setDeleting(chapter.id)
    try {
      onChanged(
        await deleteExtraChapter(chapter.id, preparation.workflow.stateRevision)
      )
    } catch (error) {
      onError(error instanceof Error ? error.message : "番外未能删除")
    } finally {
      setDeleting(null)
    }
  }

  return (
    <Card className="mt-6">
      <CardHeader>
        <CardTitle>番外</CardTitle>
        <CardDescription>
          正文完本标记会保留。新番外将从章节指导开始，并沿用现有的细纲、正文和终稿审核流程。
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        {extras.length > 0 ? (
          <ul className="space-y-2" aria-label="番外列表">
            {extras.map((chapter) => {
              const busy = deleting === chapter.id
              return (
                <li
                  key={chapter.id}
                  className="flex items-center justify-between gap-2 rounded-xl border border-border bg-background/60 px-3 py-2 text-sm"
                >
                  <span className="min-w-0">
                    <span className="block truncate font-medium">
                      {chapter.label}
                    </span>
                    <span className="block text-xs text-muted-foreground">
                      {chapter.status === "approved"
                        ? "已通过"
                        : chapter.status === "in_progress"
                          ? "撰写中"
                          : "未开始"}
                    </span>
                  </span>
                  <Button
                    type="button"
                    variant={
                      confirmingDelete === chapter.id ? "destructive" : "ghost"
                    }
                    size="sm"
                    disabled={busy}
                    onClick={() => void handleDelete(chapter)}
                  >
                    {busy ? (
                      <LoaderCircle
                        className="animate-spin"
                        aria-hidden="true"
                      />
                    ) : (
                      <Trash2 aria-hidden="true" />
                    )}
                    {confirmingDelete === chapter.id ? "确认删除" : "删除"}
                  </Button>
                </li>
              )
            })}
          </ul>
        ) : null}

        <form onSubmit={handleSubmit}>
          <FieldGroup>
            <Field data-disabled={submitting || undefined}>
              <FieldLabel htmlFor="extra-chapter-title">新增番外</FieldLabel>
              <Input
                id="extra-chapter-title"
                value={title}
                onChange={(event) => setTitle(event.target.value)}
                maxLength={200}
                required
                disabled={submitting}
                placeholder="例如：番外一 · 雨停之后"
                aria-describedby="extra-chapter-description"
              />
              <FieldDescription id="extra-chapter-description">
                番外不改变已经审核通过的正文章节计划，也不会重新打开正文完本确认。
              </FieldDescription>
            </Field>
          </FieldGroup>
          <CardFooter className="mt-4 justify-end px-0">
            <Button type="submit" disabled={submitting || !title.trim()}>
              {submitting ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <Plus data-icon="inline-start" aria-hidden="true" />
              )}
              {submitting ? "正在创建…" : "创建番外"}
            </Button>
          </CardFooter>
        </form>
      </CardContent>
    </Card>
  )
}
