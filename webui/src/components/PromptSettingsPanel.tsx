import { useCallback, useEffect, useState } from "react"
import { CircleAlert, CircleCheck, Eye, RotateCcw, Save } from "lucide-react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Spinner } from "@/components/ui/spinner"
import {
  ApiError,
  createCommandId,
  getProjectPrompts,
  getSystemPrompts,
  previewPrompt,
  resetProjectPromptOverride,
  resetSystemPromptOverride,
  saveProjectPromptOverride,
  saveSystemPromptOverride,
  type PromptCatalogEntry,
  type PromptSource,
} from "@/lib/api"

type Scope = "system" | "project"

type Notice =
  | { tone: "success"; message: string }
  | { tone: "error"; message: string }
  | null

interface PromptSettingsPanelProps {
  /** 当前选中项目；为空时只能编辑系统级覆盖。 */
  projectId: string | null
  refreshToken?: number
}

interface CatalogState {
  scope: Scope
  projectId: string | null
  entries: PromptCatalogEntry[]
  failed: boolean
}

const sourceLabels: Record<PromptSource, string> = {
  builtin: "代码内建",
  system: "系统覆盖",
  project: "项目覆盖",
}

const textareaClass =
  "mt-1.5 min-h-40 w-full rounded-xl border border-input bg-background p-3 font-mono text-xs leading-5 transition outline-none placeholder:text-muted-foreground focus:border-ring focus:ring-3 focus:ring-ring/20"

function draftFor(
  entry: PromptCatalogEntry,
  scope: Scope
): { system: string; user: string } {
  const override =
    scope === "system" ? entry.systemOverride : entry.projectOverride
  if (override) return { system: override.system, user: override.user }
  // 没有覆盖时从当前生效内容起步：项目作用域已包含系统覆盖的回退结果。
  return { system: entry.effectiveSystem, user: entry.effectiveUser }
}

function overrideRevision(
  entry: PromptCatalogEntry,
  scope: Scope
): string | null {
  const override =
    scope === "system" ? entry.systemOverride : entry.projectOverride
  return override ? override.revision : null
}

function messageFor(error: unknown, fallback: string) {
  if (error instanceof ApiError) return error.message
  if (error instanceof Error) return error.message
  return fallback
}

/**
 * 系统级与项目级提示词覆盖的编辑器。
 *
 * 目录、草稿、预览都以「作用域 + 提示词键」为身份保存在状态里：切换作用域或键
 * 时直接由新键派生，不需要在副作用里同步重置，也不会把上一份草稿显示到另一处。
 */
export function PromptSettingsPanel({
  projectId,
  refreshToken = 0,
}: PromptSettingsPanelProps) {
  const [scope, setScope] = useState<Scope>("system")
  const [catalog, setCatalog] = useState<CatalogState | null>(null)
  const [selectedKey, setSelectedKey] = useState<string | null>(null)
  const [draft, setDraft] = useState<{
    key: string
    system: string
    user: string
  } | null>(null)
  const [preview, setPreview] = useState<{
    key: string
    system: string
    user: string
  } | null>(null)
  const [expanded, setExpanded] = useState<{
    key: string
    name: string
  } | null>(null)
  const [refreshTick, setRefreshTick] = useState(0)
  const [notice, setNotice] = useState<Notice>(null)
  const [pending, setPending] = useState<"save" | "reset" | "preview" | null>(
    null
  )

  const activeScope: Scope = projectId ? scope : "system"
  const requestedProject = activeScope === "project" ? projectId : null

  useEffect(() => {
    let isCurrent = true
    const request =
      activeScope === "project"
        ? getProjectPrompts(projectId ?? "")
        : getSystemPrompts()
    request
      .then((entries) => {
        if (!isCurrent) return
        setCatalog({
          scope: activeScope,
          projectId: requestedProject,
          entries,
          failed: false,
        })
      })
      .catch((error: unknown) => {
        if (!isCurrent) return
        setCatalog({
          scope: activeScope,
          projectId: requestedProject,
          entries: [],
          failed: true,
        })
        setNotice({
          tone: "error",
          message: messageFor(error, "读取提示词目录失败"),
        })
      })
    return () => {
      isCurrent = false
    }
  }, [activeScope, requestedProject, projectId, refreshToken, refreshTick])

  const entries =
    catalog &&
    catalog.scope === activeScope &&
    catalog.projectId === requestedProject
      ? catalog.entries
      : null

  const entry =
    entries?.find((item) => item.key === selectedKey) ?? entries?.[0] ?? null
  const draftKey = entry ? `${activeScope}:${entry.key}` : null
  const currentDraft =
    draft && draft.key === draftKey
      ? draft
      : entry && draftKey
        ? { key: draftKey, ...draftFor(entry, activeScope) }
        : null
  const currentPreview =
    preview && preview.key === draftKey ? preview : null
  const currentExpanded =
    expanded && expanded.key === draftKey ? expanded.name : null
  const hasOverride = entry
    ? overrideRevision(entry, activeScope) !== null
    : false

  const refresh = useCallback(() => {
    setRefreshTick((current) => current + 1)
  }, [])

  async function handleSave() {
    if (!entry || !currentDraft) return
    setPending("save")
    setNotice(null)
    try {
      const revision = overrideRevision(entry, activeScope) ?? "0"
      if (activeScope === "system") {
        await saveSystemPromptOverride(
          entry.key,
          revision,
          currentDraft.system,
          currentDraft.user,
          createCommandId()
        )
      } else if (projectId) {
        await saveProjectPromptOverride(
          projectId,
          entry.key,
          revision,
          currentDraft.system,
          currentDraft.user,
          createCommandId()
        )
      }
      setNotice({ tone: "success", message: "覆盖已保存，后续请求会立即使用。" })
      setDraft(null)
      setPreview(null)
      refresh()
    } catch (error) {
      setNotice({ tone: "error", message: messageFor(error, "保存提示词覆盖失败") })
    } finally {
      setPending(null)
    }
  }

  async function handleReset() {
    if (!entry) return
    const revision = overrideRevision(entry, activeScope)
    if (revision === null) return
    setPending("reset")
    setNotice(null)
    try {
      if (activeScope === "system") {
        await resetSystemPromptOverride(entry.key, revision, createCommandId())
      } else if (projectId) {
        await resetProjectPromptOverride(
          projectId,
          entry.key,
          revision,
          createCommandId()
        )
      }
      setNotice({
        tone: "success",
        message:
          activeScope === "system"
            ? "系统覆盖已重置，回到代码内建内容。"
            : "项目覆盖已重置，回到系统配置。",
      })
      setDraft(null)
      setPreview(null)
      refresh()
    } catch (error) {
      setNotice({ tone: "error", message: messageFor(error, "重置提示词覆盖失败") })
    } finally {
      setPending(null)
    }
  }

  async function handlePreview() {
    if (!entry || !currentDraft) return
    setPending("preview")
    setNotice(null)
    try {
      const rendered = await previewPrompt({
        promptKey: entry.key,
        projectId: requestedProject ?? undefined,
        system: currentDraft.system,
        user: currentDraft.user,
      })
      setPreview({
        key: currentDraft.key,
        system: rendered.system,
        user: rendered.user,
      })
    } catch (error) {
      setPreview(null)
      setNotice({ tone: "error", message: messageFor(error, "预览失败") })
    } finally {
      setPending(null)
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>提示词</CardTitle>
        <CardDescription>
          覆盖内建提示词的 system / user 成对内容；项目覆盖优先于系统覆盖。
        </CardDescription>
        <CardAction>
          <div className="flex gap-1" role="group" aria-label="提示词作用域">
            <Button
              size="sm"
              variant={activeScope === "system" ? "default" : "outline"}
              onClick={() => setScope("system")}
            >
              系统
            </Button>
            <Button
              size="sm"
              variant={activeScope === "project" ? "default" : "outline"}
              disabled={!projectId}
              title={projectId ? undefined : "先选择一个项目"}
              onClick={() => setScope("project")}
            >
              当前项目
            </Button>
          </div>
        </CardAction>
      </CardHeader>
      <CardContent className="space-y-4">
        {notice && (
          <Alert variant={notice.tone === "error" ? "destructive" : "default"}>
            {notice.tone === "error" ? (
              <CircleAlert aria-hidden="true" />
            ) : (
              <CircleCheck aria-hidden="true" />
            )}
            <AlertTitle>
              {notice.tone === "error" ? "操作未完成" : "已保存"}
            </AlertTitle>
            <AlertDescription>{notice.message}</AlertDescription>
          </Alert>
        )}

        {entries === null ? (
          catalog?.failed ? (
            <p className="text-sm text-muted-foreground">
              未能读取提示词目录；请修正上方提示后重试。
            </p>
          ) : (
            <p className="flex items-center gap-2 text-sm text-muted-foreground">
              <Spinner /> 正在读取提示词目录…
            </p>
          )
        ) : (
          <>
            <div
              className="flex flex-wrap gap-1"
              role="listbox"
              aria-label="内建提示词"
            >
              {entries.map((item) => {
                const isSelected = entry?.key === item.key
                return (
                  <button
                    key={item.key}
                    type="button"
                    role="option"
                    aria-selected={isSelected}
                    onClick={() => setSelectedKey(item.key)}
                    className={`rounded-full border px-2.5 py-1 font-mono text-xs transition ${
                      isSelected
                        ? "border-primary bg-primary text-primary-foreground"
                        : "border-border bg-background hover:border-foreground/25"
                    }`}
                  >
                    {item.key}
                  </button>
                )
              })}
            </div>

            {entry && currentDraft && (
              <div className="space-y-4 rounded-xl border border-border p-4">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-mono text-sm">{entry.key}</span>
                  <Badge variant="secondary">
                    {sourceLabels[entry.effectiveSource]}
                  </Badge>
                  <span className="text-xs text-muted-foreground">
                    {hasOverride ? "已有覆盖" : "尚未覆盖"}
                  </span>
                </div>

                <div>
                  <p className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">
                    变量说明
                  </p>
                  <ul className="mt-2 space-y-1">
                    {entry.variables.map((variable) => (
                      <li key={variable.name} className="text-sm">
                        <span className="flex items-center gap-2">
                          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
                            {`{{${variable.name}}}`}
                          </code>
                          <button
                            type="button"
                            aria-expanded={currentExpanded === variable.name}
                            aria-label={`查看 ${variable.name} 的说明`}
                            onClick={() =>
                              setExpanded(
                                currentExpanded === variable.name
                                  ? null
                                  : {
                                      key: currentDraft.key,
                                      name: variable.name,
                                    }
                              )
                            }
                            className="flex size-5 items-center justify-center rounded-full border border-border text-xs text-muted-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
                          >
                            ?
                          </button>
                          {variable.required ? (
                            <span className="text-xs text-muted-foreground">
                              必填
                            </span>
                          ) : null}
                        </span>
                        {currentExpanded === variable.name && (
                          <dl className="mt-1 space-y-0.5 pl-1 text-xs text-muted-foreground">
                            <div>{variable.description}</div>
                            <div>类型：{variable.valueType}</div>
                            <div>示例：{variable.example}</div>
                            <div>来源：{variable.source}</div>
                          </dl>
                        )}
                      </li>
                    ))}
                  </ul>
                </div>

                <label className="block text-sm font-medium">
                  system 内容
                  <textarea
                    value={currentDraft.system}
                    onChange={(event) =>
                      setDraft({
                        key: currentDraft.key,
                        system: event.target.value,
                        user: currentDraft.user,
                      })
                    }
                    className={textareaClass}
                    spellCheck={false}
                  />
                </label>
                <label className="block text-sm font-medium">
                  user 内容
                  <textarea
                    value={currentDraft.user}
                    onChange={(event) =>
                      setDraft({
                        key: currentDraft.key,
                        system: currentDraft.system,
                        user: event.target.value,
                      })
                    }
                    className={textareaClass}
                    spellCheck={false}
                  />
                </label>

                <div className="flex flex-wrap gap-2">
                  <Button
                    onClick={() => void handleSave()}
                    disabled={pending !== null}
                  >
                    {pending === "save" ? <Spinner /> : <Save aria-hidden="true" />}
                    保存覆盖
                  </Button>
                  <Button
                    variant="outline"
                    onClick={() => void handlePreview()}
                    disabled={pending !== null}
                  >
                    {pending === "preview" ? (
                      <Spinner />
                    ) : (
                      <Eye aria-hidden="true" />
                    )}
                    用示例值预览
                  </Button>
                  <Button
                    variant="outline"
                    onClick={() => void handleReset()}
                    disabled={pending !== null || !hasOverride}
                    title={hasOverride ? undefined : "当前没有覆盖可重置"}
                  >
                    {pending === "reset" ? (
                      <Spinner />
                    ) : (
                      <RotateCcw aria-hidden="true" />
                    )}
                    重置
                  </Button>
                </div>

                {currentPreview && (
                  <div className="space-y-2 rounded-xl bg-muted/40 p-3">
                    <p className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">
                      渲染预览（示例值）
                    </p>
                    <pre className="max-h-56 overflow-auto text-xs whitespace-pre-wrap">
                      {currentPreview.system}
                    </pre>
                    <pre className="max-h-56 overflow-auto text-xs whitespace-pre-wrap">
                      {currentPreview.user}
                    </pre>
                  </div>
                )}
              </div>
            )}
          </>
        )}
      </CardContent>
    </Card>
  )
}
