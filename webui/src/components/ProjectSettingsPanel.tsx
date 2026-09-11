import { useCallback, useEffect, useRef, useState, type FormEvent } from "react"
import { CircleAlert, CircleCheck, RefreshCw, Save, SlidersHorizontal } from "lucide-react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardAction,
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
  ApiError,
  createCommandId,
  getProjectSettings,
  getSystemSettings,
  saveProjectSettings,
  type ProjectSettings,
  type ThinkingLevel,
} from "@/lib/api"

interface ProjectSettingsPanelProps {
  /** 当前选中项目；为空时没有可编辑的项目设置。 */
  projectId: string | null
  refreshToken?: number
}

interface Draft {
  projectId: string
  modelId: string
  thinkingLevel: ThinkingLevel
  auxiliaryToolBudget: string
  revision: string
}

type Notice =
  | { tone: "success"; message: string }
  | { tone: "error"; message: string }
  | null

const thinkingLevels: Array<{ value: ThinkingLevel; label: string }> = [
  { value: "off", label: "关闭（不发送思考参数）" },
  { value: "low", label: "低" },
  { value: "medium", label: "中" },
  { value: "high", label: "高" },
]

const selectClass =
  "mt-1.5 h-10 w-full rounded-xl border border-input bg-background px-3 text-sm transition outline-none focus:border-ring focus:ring-3 focus:ring-ring/20 disabled:cursor-not-allowed disabled:opacity-50"

function draftFor(projectId: string, settings: ProjectSettings): Draft {
  return {
    projectId,
    modelId: settings.modelId ?? "",
    thinkingLevel: settings.thinkingLevel,
    auxiliaryToolBudget: settings.auxiliaryToolBudget,
    revision: settings.revision,
  }
}

function messageFor(error: unknown, fallback: string) {
  if (error instanceof ApiError) return error.message
  if (error instanceof Error) return error.message
  return fallback
}

/**
 * 项目级创作设置：模型引用、思考等级与辅助工具预算。
 *
 * 草稿以「项目 id + 修订」为身份保存：切换项目时旧草稿自然失效，保存成功后回写
 * 服务端返回的修订，因此并发修订冲突会直接以提示呈现，而不是静默覆盖。
 */
export function ProjectSettingsPanel({
  projectId,
  refreshToken = 0,
}: ProjectSettingsPanelProps) {
  const [draft, setDraft] = useState<Draft | null>(null)
  const [thinkingSupported, setThinkingSupported] = useState<boolean | null>(
    null
  )
  const [saving, setSaving] = useState(false)
  const [isDirty, setIsDirty] = useState(false)
  const [refreshTick, setRefreshTick] = useState(0)
  const [notice, setNotice] = useState<Notice>(null)
  const saveCommandIdRef = useRef<string | null>(null)
  // 有未保存编辑时不接受后台刷新，避免设置事件到达时丢掉用户正在输入的内容。
  const editRef = useRef(false)
  const loadedProjectRef = useRef<string | null>(null)

  const updateDraft = useCallback((update: (current: Draft) => Draft) => {
    editRef.current = true
    setIsDirty(true)
    setDraft((current) => (current ? update(current) : current))
  }, [])

  useEffect(() => {
    if (!projectId) return
    let isCurrent = true
    const projectChanged = loadedProjectRef.current !== projectId
    loadedProjectRef.current = projectId
    // 命令 ID 属于某个具体项目的某次写入，切换项目时必须作废。
    if (projectChanged) saveCommandIdRef.current = null
    Promise.all([getProjectSettings(projectId), getSystemSettings()])
      .then(([settings, system]) => {
        if (!isCurrent) return
        setThinkingSupported(system.writingModel?.capabilities.thinking ?? null)
        if (projectChanged || !editRef.current) {
          setDraft(draftFor(projectId, settings))
          setIsDirty(false)
          editRef.current = false
        }
        setNotice(null)
      })
      .catch((error: unknown) => {
        if (!isCurrent) return
        if (projectChanged || !editRef.current) {
          setDraft(null)
          setIsDirty(false)
          editRef.current = false
        }
        setNotice({
          tone: "error",
          message: messageFor(error, "无法读取项目设置"),
        })
      })
    return () => {
      isCurrent = false
    }
  }, [projectId, refreshToken, refreshTick])

  async function handleSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!draft) return
    setSaving(true)
    setNotice(null)
    const commandId = saveCommandIdRef.current ?? createCommandId()
    saveCommandIdRef.current = commandId

    try {
      const trimmedModelId = draft.modelId.trim()
      const saved = await saveProjectSettings(
        draft.projectId,
        {
          expectedRevision: draft.revision,
          modelId: trimmedModelId === "" ? null : trimmedModelId,
          thinkingLevel: draft.thinkingLevel,
          auxiliaryToolBudget: draft.auxiliaryToolBudget,
        },
        commandId
      )
      setDraft(draftFor(draft.projectId, saved))
      setIsDirty(false)
      editRef.current = false
      saveCommandIdRef.current = null
      setNotice({ tone: "success", message: "项目设置已保存。" })
    } catch (error) {
      if (error instanceof ApiError) saveCommandIdRef.current = null
      setNotice({ tone: "error", message: messageFor(error, "项目设置未保存") })
    } finally {
      setSaving(false)
    }
  }

  const activeDraft = draft && draft.projectId === projectId ? draft : null
  const thinkingBlocked =
    activeDraft !== null &&
    activeDraft.thinkingLevel !== "off" &&
    thinkingSupported === false

  return (
    <Card className="border border-border/80 bg-background/85 backdrop-blur">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <span className="flex size-8 items-center justify-center rounded-xl bg-primary text-primary-foreground">
            <SlidersHorizontal className="size-4" aria-hidden="true" />
          </span>
          项目设置
        </CardTitle>
        <CardDescription>
          每个项目可以覆盖写作模型、指定思考等级并限制辅助工具预算；未覆盖时沿用系统默认。
        </CardDescription>
        <CardAction>
          {activeDraft ? (
            <Badge variant="secondary">修订 {activeDraft.revision}</Badge>
          ) : (
            <Badge variant="outline">
              {projectId ? "读取中" : "未选择项目"}
            </Badge>
          )}
        </CardAction>
      </CardHeader>

      <CardContent>
        {notice && (
          <Alert
            variant={notice.tone === "error" ? "destructive" : "default"}
            className="mb-5"
          >
            {notice.tone === "error" ? (
              <CircleAlert aria-hidden="true" />
            ) : (
              <CircleCheck className="text-primary" aria-hidden="true" />
            )}
            <AlertTitle>
              {notice.tone === "error" ? "操作未完成" : "操作已完成"}
            </AlertTitle>
            <AlertDescription>{notice.message}</AlertDescription>
          </Alert>
        )}

        {!activeDraft ? (
          <p className="text-sm leading-6 text-muted-foreground">
            {projectId
              ? "正在读取项目设置…"
              : "选择或创建一个项目后即可编辑项目设置。"}
          </p>
        ) : (
          <form onSubmit={handleSave}>
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="project-model-id">模型引用</FieldLabel>
                <Input
                  id="project-model-id"
                  maxLength={256}
                  value={activeDraft.modelId}
                  onChange={(event) => {
                    const modelId = event.target.value
                    updateDraft((current) => ({ ...current, modelId }))
                  }}
                  placeholder="留空则沿用系统写作模型"
                  autoComplete="off"
                  disabled={saving}
                />
                <FieldDescription>
                  只有需要为该项目单独指定服务上的模型时才填写。
                </FieldDescription>
              </Field>

              <Field>
                <FieldLabel htmlFor="project-thinking-level">思考等级</FieldLabel>
                <select
                  id="project-thinking-level"
                  className={selectClass}
                  value={activeDraft.thinkingLevel}
                  onChange={(event) => {
                    const thinkingLevel = event.target.value as ThinkingLevel
                    updateDraft((current) => ({ ...current, thinkingLevel }))
                  }}
                  disabled={saving}
                >
                  {thinkingLevels.map((level) => (
                    <option key={level.value} value={level.value}>
                      {level.label}
                    </option>
                  ))}
                </select>
                <FieldDescription>
                  每次请求都会按当前设置重新构建；已发出的请求快照不会被改写。
                </FieldDescription>
              </Field>

              <Field>
                <FieldLabel htmlFor="project-tool-budget">
                  辅助工具预算
                </FieldLabel>
                <Input
                  id="project-tool-budget"
                  type="number"
                  min="0"
                  max="10000"
                  step="1"
                  inputMode="numeric"
                  value={activeDraft.auxiliaryToolBudget}
                  onChange={(event) => {
                    const auxiliaryToolBudget = event.target.value
                    updateDraft((current) => ({
                      ...current,
                      auxiliaryToolBudget,
                    }))
                  }}
                  disabled={saving}
                />
                <FieldDescription>
                  该项目内可供辅助工具使用的调用次数上限（0–10000）。
                </FieldDescription>
              </Field>

              {thinkingBlocked && (
                <Alert variant="destructive">
                  <CircleAlert aria-hidden="true" />
                  <AlertTitle>系统写作模型不支持思考</AlertTitle>
                  <AlertDescription>
                    请先在「写作模型」中打开「支持思考」，否则下一次写作请求会因思考等级而失败。
                  </AlertDescription>
                </Alert>
              )}

              <div className="flex flex-wrap gap-2">
                <Button type="submit" disabled={saving}>
                  {saving ? (
                    <Spinner data-icon="inline-start" aria-hidden="true" />
                  ) : (
                    <Save data-icon="inline-start" aria-hidden="true" />
                  )}
                  {saving ? "正在保存…" : "保存项目设置"}
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  onClick={() => setRefreshTick((tick) => tick + 1)}
                  disabled={saving}
                >
                  <RefreshCw data-icon="inline-start" aria-hidden="true" />
                  重新载入
                </Button>
              </div>
            </FieldGroup>
          </form>
        )}
      </CardContent>

      <CardFooter className="border-t text-xs leading-5 text-muted-foreground">
        {isDirty
          ? "有未保存的项目设置；保存后下一次请求才会使用新值。"
          : "项目设置与项目本身分开保存修订，因此保存设置不会让正在进行的稿本切换命令过期。"}
      </CardFooter>
    </Card>
  )
}
