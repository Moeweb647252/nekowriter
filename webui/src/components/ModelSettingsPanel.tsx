import { useCallback, useEffect, useRef, useState, type FormEvent } from "react"
import {
  Cable,
  CheckCircle2,
  CircleAlert,
  KeyRound,
  RefreshCw,
  Save,
  Settings2,
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
  getSystemSettings,
  saveSystemModelSettings,
  testModelConnection,
  type ModelCapabilityProfile,
  type PublicSystemModelSettings,
  type TestModelResponse,
} from "@/lib/api"

interface ModelSettingsPanelProps {
  refreshToken?: number
  /** 连接测试会落库成系统操作；调用方据此让概览重新读取。 */
  onModelOperation?: () => void
}

interface SettingsDraft {
  apiBaseUrl: string
  modelId: string
  maxContextTokens: string
  reservedOutputTokens: string
  capabilities: ModelCapabilityProfile
}

type Notice =
  | { tone: "success"; message: string }
  | { tone: "error"; message: string }
  | null

const defaultDraft: SettingsDraft = {
  apiBaseUrl: "https://api.openai.com/v1",
  modelId: "",
  maxContextTokens: "32768",
  reservedOutputTokens: "4096",
  capabilities: {
    streaming: true,
    tools: true,
    streamUsage: true,
    // 思考支持需要实际验证后才能声明，缺省为未验证。
    thinking: false,
  },
}

function draftFromSettings(
  settings: PublicSystemModelSettings | null
): SettingsDraft {
  if (!settings) return defaultDraft

  return {
    apiBaseUrl: settings.apiBaseUrl,
    modelId: settings.modelId,
    maxContextTokens: settings.maxContextTokens,
    reservedOutputTokens: settings.reservedOutputTokens,
    capabilities: settings.capabilities,
  }
}

function formatUsage(result: TestModelResponse) {
  if (!result.usage) return "服务未报告用量"

  const { inputTokens, outputTokens, totalTokens, cachedInputTokens } =
    result.usage
  const cache = cachedInputTokens ? `，缓存命中 ${cachedInputTokens}` : ""
  return `输入 ${inputTokens}，输出 ${outputTokens}，合计 ${totalTokens}${cache}`
}

export function ModelSettingsPanel({
  refreshToken = 0,
  onModelOperation,
}: ModelSettingsPanelProps) {
  const [settings, setSettings] = useState<PublicSystemModelSettings | null>(
    null
  )
  const [draft, setDraft] = useState<SettingsDraft>(defaultDraft)
  const [apiKey, setApiKey] = useState("")
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [testing, setTesting] = useState(false)
  const [verifying, setVerifying] = useState(false)
  const [isDirty, setIsDirty] = useState(false)
  const [notice, setNotice] = useState<Notice>(null)
  /**
   * 已通过一次真的携带 `reasoning_effort` 的连接测试确认过支持思考的模型 ID。
   *
   * 用模型 ID 而不是布尔值来记录：换一个模型 ID 后旧验证自动失效，切换回同一个
   * 模型 ID 时又能沿用。加载到已持久化的声明时直接以它为初值，否则重启后开关
   * 会被锁死。
   */
  const [verifiedModelId, setVerifiedModelId] = useState<string | null>(null)
  const editRef = useRef(false)
  const saveCommandIdRef = useRef<string | null>(null)

  const updateDraft = useCallback(
    (update: (current: SettingsDraft) => SettingsDraft) => {
      editRef.current = true
      setIsDirty(true)
      setDraft(update)
    },
    []
  )

  const loadSettings = useCallback(async (force = false) => {
    setLoading(true)
    try {
      const response = await getSystemSettings()
      setSettings(response.writingModel)
      if (force || !editRef.current) {
        setDraft(draftFromSettings(response.writingModel))
        setApiKey("")
        setVerifiedModelId(
          response.writingModel?.capabilities.thinking
            ? response.writingModel.modelId
            : null
        )
        setIsDirty(false)
        editRef.current = false
      }
      setNotice(null)
    } catch (error) {
      setNotice({
        tone: "error",
        message: error instanceof Error ? error.message : "无法读取模型设置",
      })
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void loadSettings()
  }, [loadSettings, refreshToken])

  async function handleSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setSaving(true)
    setNotice(null)
    const commandId = saveCommandIdRef.current ?? createCommandId()
    saveCommandIdRef.current = commandId

    try {
      const response = await saveSystemModelSettings(
        {
          expectedRevision: settings?.revision ?? "0",
          apiBaseUrl: draft.apiBaseUrl,
          modelId: draft.modelId,
          capabilities: draft.capabilities,
          maxContextTokens: draft.maxContextTokens,
          reservedOutputTokens: draft.reservedOutputTokens,
          ...(apiKey ? { apiKey } : {}),
        },
        commandId
      )
      const saved = response.writingModel
      setSettings(saved)
      setDraft(draftFromSettings(saved))
      setApiKey("")
      setIsDirty(false)
      editRef.current = false
      saveCommandIdRef.current = null
      setNotice({ tone: "success", message: "模型设置已保存到本机。" })
    } catch (error) {
      if (error instanceof ApiError) saveCommandIdRef.current = null
      setNotice({
        tone: "error",
        message: error instanceof Error ? error.message : "模型设置未保存",
      })
    } finally {
      setSaving(false)
    }
  }

  async function handleTest() {
    setTesting(true)
    setNotice(null)
    const modelId = draft.modelId.trim()
    try {
      const result = await testModelConnection({
        modelId,
        capabilities: draft.capabilities,
      })
      const received = result.contentReceived
        ? "已收到模型响应"
        : "请求完成，但服务没有返回文本"
      if (result.thinkingProbed) setVerifiedModelId(modelId)
      const probed = result.thinkingProbed
        ? "；已按声明携带 reasoning_effort，思考支持因此通过验证"
        : ""
      setNotice({
        tone: "success",
        message: `${received}；${formatUsage(result)}${probed}。`,
      })
    } catch (error) {
      setNotice({
        tone: "error",
        message: error instanceof Error ? error.message : "模型连接测试失败",
      })
    } finally {
      setTesting(false)
      // 无论成功或失败，这次请求都已经落库成系统操作。
      onModelOperation?.()
    }
  }

  /**
   * 验证当前模型是否真的接受 `reasoning_effort`。
   *
   * 与普通连接测试的区别是它总是带上思考参数：只有这样才能把「支持思考」从用户
   * 的猜测变成实测过的声明。
   */
  async function handleVerifyThinking() {
    setVerifying(true)
    setNotice(null)
    const modelId = draft.modelId.trim()
    try {
      await testModelConnection({
        modelId,
        capabilities: { ...draft.capabilities, thinking: true },
      })
      setVerifiedModelId(modelId)
      setNotice({
        tone: "success",
        message: `模型 ${modelId} 接受了 reasoning_effort，现在可以声明支持思考并保存。`,
      })
    } catch (error) {
      setVerifiedModelId(null)
      setNotice({
        tone: "error",
        message: error instanceof Error ? error.message : "思考能力验证失败",
      })
    } finally {
      setVerifying(false)
      onModelOperation?.()
    }
  }

  const canTest = !!settings && !isDirty && !saving && !loading
  const thinkingVerified =
    verifiedModelId !== null && verifiedModelId === draft.modelId.trim()

  return (
    <Card className="border border-border/80 bg-background/85 backdrop-blur">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <span className="flex size-8 items-center justify-center rounded-xl bg-primary text-primary-foreground">
            <Settings2 className="size-4" aria-hidden="true" />
          </span>
          写作模型
        </CardTitle>
        <CardDescription>
          配置一个已验证的 Chat Completions 服务，供后续写作请求使用。
        </CardDescription>
        <CardAction>
          {settings?.credentialConfigured ? (
            <Badge variant="secondary">凭据已配置</Badge>
          ) : (
            <Badge variant="outline">尚未配置</Badge>
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
              <CheckCircle2 className="text-primary" aria-hidden="true" />
            )}
            <AlertTitle>
              {notice.tone === "error" ? "操作未完成" : "操作已完成"}
            </AlertTitle>
            <AlertDescription>{notice.message}</AlertDescription>
          </Alert>
        )}

        <form onSubmit={handleSave}>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="writing-model-api-base-url">
                服务基础地址
              </FieldLabel>
              <Input
                id="writing-model-api-base-url"
                required
                type="url"
                value={draft.apiBaseUrl}
                onChange={(event) =>
                  updateDraft((current) => ({
                    ...current,
                    apiBaseUrl: event.target.value,
                  }))
                }
                placeholder="https://api.openai.com/v1"
                autoComplete="url"
                disabled={loading || saving}
              />
              <FieldDescription>
                填写 API 前缀，不要填写完整的 chat/completions 地址。
              </FieldDescription>
            </Field>

            <Field>
              <FieldLabel htmlFor="writing-model-id">模型 ID</FieldLabel>
              <Input
                id="writing-model-id"
                required
                maxLength={256}
                value={draft.modelId}
                onChange={(event) => {
                  const modelId = event.target.value
                  updateDraft((current) => ({
                    ...current,
                    modelId,
                    // 换模型 ID 后旧的思考验证不再适用，该声明必须一并清除。
                    capabilities:
                      modelId.trim() === verifiedModelId
                        ? current.capabilities
                        : { ...current.capabilities, thinking: false },
                  }))
                }}
                placeholder="例如：gpt-5"
                autoComplete="off"
                disabled={loading || saving}
              />
            </Field>

            <Field>
              <FieldLabel htmlFor="writing-model-api-key">
                <KeyRound className="size-3.5" aria-hidden="true" />
                API 密钥
              </FieldLabel>
              <Input
                id="writing-model-api-key"
                type="password"
                value={apiKey}
                onChange={(event) => {
                  setApiKey(event.target.value)
                  editRef.current = true
                  setIsDirty(true)
                }}
                placeholder={
                  settings?.credentialConfigured
                    ? "留空即可保留已保存的密钥"
                    : "输入服务密钥"
                }
                autoComplete="new-password"
                disabled={loading || saving}
              />
              <FieldDescription>
                密钥只保存在本机数据库的模型设置里，只在构建请求前读出；请求快照、事件与日志都不写密钥。
              </FieldDescription>
            </Field>

            <div className="grid gap-4 sm:grid-cols-2">
              <Field>
                <FieldLabel htmlFor="writing-model-context-tokens">
                  上下文上限
                </FieldLabel>
                <Input
                  id="writing-model-context-tokens"
                  required
                  type="number"
                  min="1"
                  step="1"
                  inputMode="numeric"
                  value={draft.maxContextTokens}
                  onChange={(event) =>
                    updateDraft((current) => ({
                      ...current,
                      maxContextTokens: event.target.value,
                    }))
                  }
                  disabled={loading || saving}
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="writing-model-output-tokens">
                  预留输出 Token
                </FieldLabel>
                <Input
                  id="writing-model-output-tokens"
                  required
                  type="number"
                  min="1"
                  step="1"
                  inputMode="numeric"
                  value={draft.reservedOutputTokens}
                  onChange={(event) =>
                    updateDraft((current) => ({
                      ...current,
                      reservedOutputTokens: event.target.value,
                    }))
                  }
                  disabled={loading || saving}
                />
              </Field>
            </div>

            <ModelCapabilitiesEditor
              idPrefix="writing-model"
              value={draft.capabilities}
              onChange={(capabilities) =>
                updateDraft((current) => ({ ...current, capabilities }))
              }
              disabled={loading || saving}
              thinking={{
                verified: thinkingVerified,
                verifying,
                canVerify: canTest,
                onVerify: () => void handleVerifyThinking(),
                blockedReason: isDirty
                  ? "请先保存设置，验证会针对已保存的模型 ID 发出请求。"
                  : null,
              }}
            />

            <div className="flex flex-wrap gap-2">
              <Button type="submit" disabled={loading || saving}>
                {saving ? (
                  <Spinner data-icon="inline-start" aria-hidden="true" />
                ) : (
                  <Save data-icon="inline-start" aria-hidden="true" />
                )}
                {saving ? "正在保存…" : "保存模型设置"}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={handleTest}
                disabled={!canTest || testing || verifying}
              >
                {testing ? (
                  <Spinner data-icon="inline-start" aria-hidden="true" />
                ) : (
                  <Cable data-icon="inline-start" aria-hidden="true" />
                )}
                {testing ? "正在测试…" : "测试真实连接"}
              </Button>
              <Button
                type="button"
                variant="ghost"
                onClick={() => void loadSettings(true)}
                disabled={loading || saving || testing || verifying}
              >
                <RefreshCw data-icon="inline-start" aria-hidden="true" />
                重新载入
              </Button>
            </div>
          </FieldGroup>
        </form>
      </CardContent>

      <CardFooter className="border-t text-xs leading-5 text-muted-foreground">
        {isDirty
          ? "有未保存的设置。保存后才能测试该配置。"
          : "连接测试会实际请求所配置的模型并显示用量；「验证思考支持」会额外带上 reasoning_effort，只有验证通过才能声明支持思考。"}
      </CardFooter>
    </Card>
  )
}
