import { useCallback, useEffect, useRef, useState, type FormEvent } from "react"
import {
  CircleAlert,
  CircleCheck,
  RefreshCw,
  Save,
  SlidersHorizontal,
} from "lucide-react"

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
import { ModelCapabilitiesEditor } from "@/components/ModelCapabilitiesEditor"
import {
  ApiError,
  createCommandId,
  getProjectSettings,
  getSystemSettings,
  saveProjectSettings,
  testModelConnection,
  type ModelCapabilityProfile,
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
  /** 空字符串表示沿用系统写作模型。 */
  modelId: string
  capabilities: ModelCapabilityProfile
  thinkingLevel: ThinkingLevel
  auxiliaryToolBudget: string
  revision: string
}

type Notice =
  | { tone: "success"; message: string }
  | { tone: "error"; message: string }
  | null

/** 系统设置尚未加载时的兜底能力，与系统面板的默认值保持同一份。 */
const fallbackCapabilities: ModelCapabilityProfile = {
  streaming: true,
  tools: true,
  structuredOutput: true,
  streamUsage: true,
  thinking: false,
}

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
    modelId: settings.model?.modelId ?? "",
    capabilities: settings.model?.capabilities ?? fallbackCapabilities,
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
 * 模型引用是一个整体：模型 ID 与它自己的能力资料一起保存。切换模型 ID 会重置能力
 * 声明（系统模型的声明只作为起点），因为另一张模型的能力不能沿用；「支持思考」还
 * 必须对这个具体模型真的验证过一次，否则面板不会允许声明。
 *
 * 草稿以「项目 id + 修订」为身份保存：切换项目时旧草稿自然失效，保存成功后回写
 * 服务端返回的修订，因此并发修订冲突会直接以提示呈现，而不是静默覆盖。
 */
export function ProjectSettingsPanel({
  projectId,
  refreshToken = 0,
}: ProjectSettingsPanelProps) {
  const [draft, setDraft] = useState<Draft | null>(null)
  const [systemCapabilities, setSystemCapabilities] =
    useState<ModelCapabilityProfile | null>(null)
  /**
   * 已通过连接测试确认过思考能力的项目模型：记录模型 ID 与被验证的能力资料。
   *
   * 用「模型 ID + 当时的能力资料」而不是布尔值，是为了让换模型 ID 自动使验证失效、
   * 换回同一个模型 ID 又能沿用，同时不至于在重启后把已持久化的声明锁死。
   */
  const [verified, setVerified] = useState<{
    modelId: string
    capabilities: ModelCapabilityProfile
  } | null>(null)
  const [verifying, setVerifying] = useState(false)
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
        setSystemCapabilities(system.writingModel?.capabilities ?? null)
        // 已持久化的思考声明视为已验证，否则重启后开关会被锁死。
        setVerified(
          settings.model?.capabilities.thinking
            ? {
                modelId: settings.model.modelId,
                capabilities: settings.model.capabilities,
              }
            : null
        )
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

  /**
   * 更换模型 ID 时重新确定能力声明。
   *
   * 从「无覆盖」切到某个模型时，以系统模型的声明为起点（同一服务上的模型通常相近），
   * 用户仍可逐项修改；切回已验证的那个模型时恢复验证时的能力资料；其余情况一律
   * 清除思考声明，因为它属于上一张模型。
   */
  function capabilitiesForModelChange(
    nextModelId: string
  ): ModelCapabilityProfile {
    if (verified && verified.modelId === nextModelId)
      return verified.capabilities
    const base = systemCapabilities ?? fallbackCapabilities
    return { ...base, thinking: false }
  }

  async function handleVerifyThinking() {
    if (!draft) return
    setVerifying(true)
    setNotice(null)
    const modelId = draft.modelId.trim()
    const capabilities = { ...draft.capabilities, thinking: true }
    try {
      await testModelConnection({ modelId, capabilities })
      setVerified({ modelId, capabilities })
      setNotice({
        tone: "success",
        message: `模型 ${modelId} 接受了 reasoning_effort，现在可以声明支持思考并保存。`,
      })
    } catch (error) {
      setVerified(null)
      setNotice({
        tone: "error",
        message: messageFor(error, "思考能力验证失败"),
      })
    } finally {
      setVerifying(false)
    }
  }

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
          model:
            trimmedModelId === ""
              ? null
              : {
                  modelId: trimmedModelId,
                  capabilities: draft.capabilities,
                },
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
  const hasModelOverride =
    activeDraft !== null && activeDraft.modelId.trim() !== ""
  const thinkingVerified =
    activeDraft !== null &&
    verified !== null &&
    verified.modelId === activeDraft.modelId.trim()
  // 没有项目模型覆盖时，思考等级校验的是系统写作模型的能力。
  const effectiveThinking = hasModelOverride
    ? activeDraft.capabilities.thinking
    : (systemCapabilities?.thinking ?? false)
  const thinkingBlocked =
    activeDraft !== null &&
    activeDraft.thinkingLevel !== "off" &&
    !effectiveThinking

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
                    updateDraft((current) => ({
                      ...current,
                      modelId,
                      capabilities: capabilitiesForModelChange(modelId.trim()),
                    }))
                  }}
                  placeholder="留空则沿用系统写作模型"
                  autoComplete="off"
                  disabled={saving}
                />
                <FieldDescription>
                  只有需要为该项目单独指定服务上的模型时才填写。模型引用只换模型，
                  服务地址与凭据仍来自系统设置。
                </FieldDescription>
              </Field>

              {hasModelOverride && (
                <ModelCapabilitiesEditor
                  idPrefix="project-model"
                  value={activeDraft.capabilities}
                  onChange={(capabilities) =>
                    updateDraft((current) => ({ ...current, capabilities }))
                  }
                  disabled={saving}
                  thinking={{
                    verified: thinkingVerified,
                    verifying,
                    canVerify: !saving && !verifying,
                    onVerify: () => void handleVerifyThinking(),
                    blockedReason: null,
                  }}
                />
              )}

              <Field>
                <FieldLabel htmlFor="project-thinking-level">
                  思考等级
                </FieldLabel>
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
                  <AlertTitle>当前模型不支持思考</AlertTitle>
                  <AlertDescription>
                    {hasModelOverride
                      ? "项目引用的模型未声明支持思考。请为它验证并打开「支持思考」，或把思考等级设为关闭。"
                      : "系统写作模型不支持思考。请先在「写作模型」中验证并打开「支持思考」，否则下一次写作请求会因思考等级而失败。"}
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
                  disabled={saving || verifying}
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
