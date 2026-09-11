import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
} from "react"
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
  FieldContent,
  FieldDescription,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
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
    structuredOutput: true,
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

export function ModelSettingsPanel({ refreshToken = 0 }: ModelSettingsPanelProps) {
  const [settings, setSettings] = useState<PublicSystemModelSettings | null>(
    null
  )
  const [draft, setDraft] = useState<SettingsDraft>(defaultDraft)
  const [apiKey, setApiKey] = useState("")
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [testing, setTesting] = useState(false)
  const [isDirty, setIsDirty] = useState(false)
  const [notice, setNotice] = useState<Notice>(null)
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
    try {
      const result = await testModelConnection()
      const received = result.contentReceived
        ? "已收到模型响应"
        : "请求完成，但服务没有返回文本"
      setNotice({
        tone: "success",
        message: `${received}；${formatUsage(result)}。`,
      })
    } catch (error) {
      setNotice({
        tone: "error",
        message: error instanceof Error ? error.message : "模型连接测试失败",
      })
    } finally {
      setTesting(false)
    }
  }

  const canTest = !!settings && !isDirty && !saving && !loading

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
                onChange={(event) =>
                  updateDraft((current) => ({
                    ...current,
                    modelId: event.target.value,
                  }))
                }
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
                密钥只会交给本地服务，并存入 Windows 凭据管理器；数据库和请求快照不保存密钥。
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

            <FieldSet>
              <FieldLegend>模型能力</FieldLegend>
              <FieldDescription>
                写作流程会在发出请求前验证这些能力，避免把不兼容的配置当作可用服务。
              </FieldDescription>
              <Field orientation="horizontal" data-disabled={loading || saving || undefined}>
                <Switch id="writing-model-streaming" checked disabled />
                <FieldContent>
                  <FieldLabel htmlFor="writing-model-streaming">流式响应</FieldLabel>
                  <FieldDescription>写作请求必须支持流式输出。</FieldDescription>
                </FieldContent>
              </Field>
              <Field orientation="horizontal" data-disabled={loading || saving || undefined}>
                <Switch
                  id="writing-model-tools"
                  checked={draft.capabilities.tools}
                  onCheckedChange={(tools) =>
                    updateDraft((current) => ({
                      ...current,
                      capabilities: { ...current.capabilities, tools },
                    }))
                  }
                  disabled={loading || saving}
                />
                <FieldContent>
                  <FieldLabel htmlFor="writing-model-tools">工具调用</FieldLabel>
                  <FieldDescription>允许模型调用受控的写作工具。</FieldDescription>
                </FieldContent>
              </Field>
              <Field orientation="horizontal" data-disabled={loading || saving || undefined}>
                <Switch
                  id="writing-model-structured-output"
                  checked={draft.capabilities.structuredOutput}
                  onCheckedChange={(structuredOutput) =>
                    updateDraft((current) => ({
                      ...current,
                      capabilities: {
                        ...current.capabilities,
                        structuredOutput,
                      },
                    }))
                  }
                  disabled={loading || saving}
                />
                <FieldContent>
                  <FieldLabel htmlFor="writing-model-structured-output">
                    结构化提交
                  </FieldLabel>
                  <FieldDescription>允许模型提交可验证的阶段产物。</FieldDescription>
                </FieldContent>
              </Field>
              <Field orientation="horizontal" data-disabled={loading || saving || undefined}>
                <Switch
                  id="writing-model-stream-usage"
                  checked={draft.capabilities.streamUsage}
                  onCheckedChange={(streamUsage) =>
                    updateDraft((current) => ({
                      ...current,
                      capabilities: { ...current.capabilities, streamUsage },
                    }))
                  }
                  disabled={loading || saving}
                />
                <FieldContent>
                  <FieldLabel htmlFor="writing-model-stream-usage">
                    流式用量
                  </FieldLabel>
                  <FieldDescription>服务可在流结束时返回 Token 用量。</FieldDescription>
                </FieldContent>
              </Field>
              <Field orientation="horizontal" data-disabled={loading || saving || undefined}>
                <Switch
                  id="writing-model-thinking"
                  checked={draft.capabilities.thinking}
                  onCheckedChange={(thinking) =>
                    updateDraft((current) => ({
                      ...current,
                      capabilities: { ...current.capabilities, thinking },
                    }))
                  }
                  disabled={loading || saving}
                />
                <FieldContent>
                  <FieldLabel htmlFor="writing-model-thinking">
                    支持思考
                  </FieldLabel>
                  <FieldDescription>
                    确认该服务接受 reasoning_effort。项目设置里的思考等级生效前会校验此能力。
                  </FieldDescription>
                </FieldContent>
              </Field>
            </FieldSet>

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
                disabled={!canTest || testing}
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
                disabled={loading || saving || testing}
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
          : "连接测试会实际请求所配置的模型，并显示服务报告的用量。"}
      </CardFooter>
    </Card>
  )
}
