import { useState, type FormEvent } from "react"
import { Plus } from "lucide-react"

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
import { addExtraChapter, type PreparationSnapshot } from "@/lib/api"

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

  return (
    <form onSubmit={handleSubmit}>
      <Card className="mt-6">
        <CardHeader>
          <CardTitle>新增番外</CardTitle>
          <CardDescription>
            正文完本标记会保留。新番外将从章节指导开始，并沿用现有的细纲、正文和终稿审核流程。
          </CardDescription>
        </CardHeader>
        <CardContent>
          <FieldGroup>
            <Field data-disabled={submitting || undefined}>
              <FieldLabel htmlFor="extra-chapter-title">番外标题</FieldLabel>
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
        </CardContent>
        <CardFooter className="justify-end">
          <Button type="submit" disabled={submitting || !title.trim()}>
            {submitting ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <Plus data-icon="inline-start" aria-hidden="true" />
            )}
            {submitting ? "正在创建…" : "创建番外"}
          </Button>
        </CardFooter>
      </Card>
    </form>
  )
}
