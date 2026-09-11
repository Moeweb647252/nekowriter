import { useEffect, useState, type FormEvent } from "react"
import { CopyPlus, FilePlus2, GitBranch } from "lucide-react"

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
  copyVersion,
  createVersion,
  listVersions,
  selectVersion,
  type NovelVersion,
  type Project,
  type ProjectBundle,
} from "@/lib/api"

type VersionPanelProps = {
  project: Project
  selectedVersion: NovelVersion
  onChanged: (bundle: ProjectBundle) => void
  onError: (message: string) => void
}

export function VersionPanel({
  project,
  selectedVersion,
  onChanged,
  onError,
}: VersionPanelProps) {
  const [versions, setVersions] = useState<NovelVersion[]>([])
  const [emptyLabel, setEmptyLabel] = useState("")
  const [emptyTitle, setEmptyTitle] = useState("")
  const [copyLabel, setCopyLabel] = useState("")
  const [pending, setPending] = useState<
    "select" | "empty" | "copy" | null
  >(null)

  useEffect(() => {
    let isCurrent = true
    void listVersions(project.id)
      .then((nextVersions) => {
        if (isCurrent) setVersions(nextVersions)
      })
      .catch((error: unknown) => {
        if (!isCurrent) return
        onError(error instanceof Error ? error.message : "无法读取稿本列表")
      })
    return () => {
      isCurrent = false
    }
  }, [onError, project.id, selectedVersion.id])

  async function handleSelect(version: NovelVersion) {
    if (version.id === selectedVersion.id || pending) return
    setPending("select")
    try {
      onChanged(
        await selectVersion(project.id, version.id, project.revision),
      )
    } catch (error) {
      onError(error instanceof Error ? error.message : "稿本切换失败")
    } finally {
      setPending(null)
    }
  }

  async function handleCreateEmpty(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setPending("empty")
    try {
      onChanged(
        await createVersion(project.id, {
          label: emptyLabel,
          novelTitle: emptyTitle,
        }),
      )
      setEmptyLabel("")
      setEmptyTitle("")
    } catch (error) {
      onError(error instanceof Error ? error.message : "新稿本未能创建")
    } finally {
      setPending(null)
    }
  }

  async function handleCopy(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setPending("copy")
    try {
      onChanged(await copyVersion(selectedVersion.id, copyLabel))
      setCopyLabel("")
    } catch (error) {
      onError(error instanceof Error ? error.message : "稿本复制失败")
    } finally {
      setPending(null)
    }
  }

  return (
    <Card className="mt-6">
      <CardHeader>
        <CardTitle>稿本</CardTitle>
        <CardDescription>
          每个稿本拥有独立的工作流、章节和审核记录；切换只改变当前工作台，不会混写内容。
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        <div className="space-y-2" aria-label="稿本列表">
          {versions.map((version) => {
            const selected = version.id === selectedVersion.id
            return (
              <Button
                key={version.id}
                type="button"
                variant={selected ? "default" : "outline"}
                className="h-auto w-full justify-start px-3 py-3 text-left"
                disabled={pending !== null || selected}
                onClick={() => void handleSelect(version)}
              >
                <GitBranch data-icon="inline-start" aria-hidden="true" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-medium">{version.label}</span>
                  <span className="mt-0.5 block truncate text-xs opacity-75">
                    {version.novelTitle}
                    {version.sourceVersionId ? " · 复制稿" : ""}
                  </span>
                </span>
                {selected && <span className="text-xs opacity-75">当前</span>}
              </Button>
            )
          })}
        </div>

        <form onSubmit={handleCreateEmpty}>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="empty-version-label">新空稿本</FieldLabel>
              <Input
                id="empty-version-label"
                value={emptyLabel}
                onChange={(event) => setEmptyLabel(event.target.value)}
                maxLength={80}
                required
                disabled={pending !== null}
                placeholder="例如：重写版"
              />
              <FieldDescription>
                空稿本从新的小说目标和准备阶段开始，不会保留当前稿本的任何内容。
              </FieldDescription>
            </Field>
            <Field>
              <FieldLabel htmlFor="empty-version-title">小说标题</FieldLabel>
              <Input
                id="empty-version-title"
                value={emptyTitle}
                onChange={(event) => setEmptyTitle(event.target.value)}
                maxLength={200}
                required
                disabled={pending !== null}
                placeholder="新稿本的标题"
              />
            </Field>
            <Button type="submit" disabled={pending !== null}>
              {pending === "empty" ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <FilePlus2 data-icon="inline-start" aria-hidden="true" />
              )}
              创建空稿本
            </Button>
          </FieldGroup>
        </form>

        <form onSubmit={handleCopy}>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="copy-version-label">复制当前稿本</FieldLabel>
              <Input
                id="copy-version-label"
                value={copyLabel}
                onChange={(event) => setCopyLabel(event.target.value)}
                maxLength={80}
                required
                disabled={pending !== null}
                placeholder="例如：第二稿"
              />
              <FieldDescription>
                只复制当前已通过的小说目标、准备产物和章节。未通过草稿、Run、工具调用、意见、审核过程及 Token 用量不会复制。
              </FieldDescription>
            </Field>
            <Button type="submit" variant="outline" disabled={pending !== null}>
              {pending === "copy" ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <CopyPlus data-icon="inline-start" aria-hidden="true" />
              )}
              复制已通过内容
            </Button>
          </FieldGroup>
        </form>
      </CardContent>
      <CardFooter className="justify-end text-xs text-muted-foreground">
        复制后的稿本会从第一个未完成的正文章节继续；已完本的正文保持完本状态。
      </CardFooter>
    </Card>
  )
}
