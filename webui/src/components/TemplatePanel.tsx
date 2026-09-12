import { useCallback, useEffect, useState, type FormEvent } from "react"
import {
  CircleAlert,
  CircleCheck,
  Eye,
  FilePlus2,
  LoaderCircle,
  Plus,
  Save,
  Trash2,
} from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import {
  ApiError,
  createCommandId,
  createPromptTemplate,
  deletePromptTemplate,
  listPromptTemplates,
  previewPrompt,
  updatePromptTemplate,
  type PromptTemplate,
  type TemplateVariable,
} from "@/lib/api"

type Notice =
  | { tone: "success"; message: string }
  | { tone: "error"; message: string }
  | null

interface TemplateDraft {
  title: string
  body: string
  variables: TemplateVariable[]
}

const emptyVariable: TemplateVariable = {
  name: "",
  description: "",
  valueType: "text",
  required: true,
  example: "",
}

const emptyDraft: TemplateDraft = {
  title: "",
  body: "",
  variables: [],
}

function errorText(error: unknown, fallback: string) {
  return error instanceof ApiError || error instanceof Error
    ? error.message
    : fallback
}

function draftFromTemplate(template: PromptTemplate): TemplateDraft {
  return {
    title: template.title,
    body: template.body,
    variables: template.variables.map((variable) => ({ ...variable })),
  }
}

/** 把界面上的可变行整理成后端要求的声明形状，去掉完全空白的行。 */
function normalizedVariables(variables: TemplateVariable[]) {
  return variables
    .filter(
      (variable) =>
        variable.name.trim().length > 0 ||
        variable.description.trim().length > 0
    )
    .map((variable) => ({
      ...variable,
      name: variable.name.trim(),
      description: variable.description.trim(),
    }))
}

interface TemplatePanelProps {
  refreshToken?: number
}

export function TemplatePanel({ refreshToken }: TemplatePanelProps) {
  const [templates, setTemplates] = useState<PromptTemplate[]>([])
  const [editing, setEditing] = useState<{
    id: string | null
    draft: TemplateDraft
  }>({ id: null, draft: emptyDraft })
  const [loading, setLoading] = useState(false)
  const [pending, setPending] = useState<string | null>(null)
  const [notice, setNotice] = useState<Notice>(null)
  const [preview, setPreview] = useState<string | null>(null)
  const { id: selectedId, draft } = editing

  /** 用一份新列表收敛选中项与草稿；保留仍然存在的选中模板。 */
  const applyListed = useCallback((listed: PromptTemplate[]) => {
    setTemplates(listed)
    setEditing((current) => {
      const nextId =
        current.id && listed.some((template) => template.id === current.id)
          ? current.id
          : (listed[0]?.id ?? null)
      const next = listed.find((template) => template.id === nextId) ?? null
      return { id: nextId, draft: next ? draftFromTemplate(next) : emptyDraft }
    })
  }, [])

  useEffect(() => {
    let isCurrent = true
    listPromptTemplates()
      .then((listed) => {
        if (isCurrent) applyListed(listed)
      })
      .catch((error: unknown) => {
        if (isCurrent)
          setNotice({
            tone: "error",
            message: errorText(error, "无法读取模板"),
          })
      })
    return () => {
      isCurrent = false
    }
  }, [applyListed, refreshToken])

  async function handleRefresh() {
    setLoading(true)
    try {
      applyListed(await listPromptTemplates())
    } catch (error) {
      setNotice({ tone: "error", message: errorText(error, "无法读取模板") })
    } finally {
      setLoading(false)
    }
  }

  function selectTemplate(id: string | null) {
    const template = templates.find((item) => item.id === id) ?? null
    setEditing({
      id: template?.id ?? null,
      draft: template ? draftFromTemplate(template) : emptyDraft,
    })
    setPreview(null)
  }

  const selected =
    templates.find((template) => template.id === selectedId) ?? null

  async function handleSubmit(event: FormEvent) {
    event.preventDefault()
    setPending("save")
    setNotice(null)
    try {
      const payload = {
        ...draft,
        variables: normalizedVariables(draft.variables),
      }
      if (selected) {
        const updated = await updatePromptTemplate(
          selected.id,
          selected.revision,
          payload,
          createCommandId()
        )
        setTemplates((current) =>
          current.map((template) =>
            template.id === updated.id ? updated : template
          )
        )
        setEditing({ id: updated.id, draft: draftFromTemplate(updated) })
        setPreview(null)
        setNotice({ tone: "success", message: "模板已保存" })
      } else {
        const created = await createPromptTemplate(payload, createCommandId())
        setTemplates((current) => [created, ...current])
        setEditing({ id: created.id, draft: draftFromTemplate(created) })
        setPreview(null)
        setNotice({ tone: "success", message: "模板已创建" })
      }
    } catch (error) {
      setNotice({ tone: "error", message: errorText(error, "保存失败") })
    } finally {
      setPending(null)
    }
  }

  async function handleDelete() {
    if (!selected) return
    setPending("delete")
    setNotice(null)
    try {
      await deletePromptTemplate(
        selected.id,
        selected.revision,
        createCommandId()
      )
      setTemplates((current) =>
        current.filter((template) => template.id !== selected.id)
      )
      setEditing({ id: null, draft: emptyDraft })
      setPreview(null)
      setNotice({ tone: "success", message: "模板已删除" })
    } catch (error) {
      setNotice({ tone: "error", message: errorText(error, "删除失败") })
    } finally {
      setPending(null)
    }
  }

  async function handlePreview() {
    setPending("preview")
    setNotice(null)
    try {
      const response = await previewPrompt({
        kind: "template",
        body: draft.body,
        variables: normalizedVariables(draft.variables),
      })
      setPreview(response.kind === "template" ? response.rendered : null)
    } catch (error) {
      setPreview(null)
      setNotice({ tone: "error", message: errorText(error, "预览失败") })
    } finally {
      setPending(null)
    }
  }

  function updateDraft(patch: Partial<TemplateDraft>) {
    setEditing((current) => ({
      ...current,
      draft: { ...current.draft, ...patch },
    }))
  }

  function updateVariable(index: number, patch: Partial<TemplateVariable>) {
    setEditing((current) => ({
      ...current,
      draft: {
        ...current.draft,
        variables: current.draft.variables.map((variable, position) =>
          position === index ? { ...variable, ...patch } : variable
        ),
      },
    }))
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <FilePlus2 className="size-4" aria-hidden="true" />
          提示词模板
        </CardTitle>
        <CardDescription>
          独立可复用的片段，可插入章节指导或意见草稿；不会获得 system 消息权限。
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap items-center gap-2">
          <select
            aria-label="选择模板"
            className="h-9 min-w-40 flex-1 rounded-xl border border-input bg-background px-3 text-sm"
            value={selectedId ?? ""}
            onChange={(event) => selectTemplate(event.target.value || null)}
          >
            <option value="">新建模板</option>
            {templates.map((template) => (
              <option key={template.id} value={template.id}>
                {template.title}
              </option>
            ))}
          </select>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => selectTemplate(null)}
          >
            <Plus aria-hidden="true" />
            新建
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={loading}
            onClick={() => void handleRefresh()}
          >
            {loading ? <Spinner /> : null}
            刷新
          </Button>
        </div>

        <form className="space-y-3" onSubmit={handleSubmit}>
          <Input
            aria-label="模板标题"
            placeholder="模板标题"
            maxLength={200}
            required
            value={draft.title}
            onChange={(event) => updateDraft({ title: event.target.value })}
          />
          <textarea
            aria-label="模板正文"
            placeholder="正文，使用 {{variable_name}} 占位"
            required
            rows={6}
            value={draft.body}
            onChange={(event) => updateDraft({ body: event.target.value })}
            className="w-full rounded-xl border border-input bg-background p-3 font-mono text-sm outline-none focus:border-ring focus:ring-3 focus:ring-ring/20"
          />

          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <span className="text-sm font-medium">变量声明</span>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() =>
                  updateDraft({
                    variables: [...draft.variables, { ...emptyVariable }],
                  })
                }
              >
                <Plus aria-hidden="true" />
                添加变量
              </Button>
            </div>
            {draft.variables.length === 0 ? (
              <p className="text-xs text-muted-foreground">
                没有占位符时不需要声明变量；正文里出现的每个 {"{{name}}"}{" "}
                都必须先声明。
              </p>
            ) : (
              <ul className="space-y-2">
                {draft.variables.map((variable, index) => (
                  <li
                    key={`${index}-${variable.name}`}
                    className="grid gap-2 rounded-xl border border-border p-2 sm:grid-cols-2"
                  >
                    <Input
                      aria-label={`变量 ${index + 1} 名称`}
                      placeholder="name"
                      value={variable.name}
                      onChange={(event) =>
                        updateVariable(index, { name: event.target.value })
                      }
                    />
                    <Input
                      aria-label={`变量 ${index + 1} 说明`}
                      placeholder="中文说明"
                      value={variable.description}
                      onChange={(event) =>
                        updateVariable(index, {
                          description: event.target.value,
                        })
                      }
                    />
                    <Input
                      aria-label={`变量 ${index + 1} 示例`}
                      placeholder="预览用示例值"
                      value={variable.example}
                      onChange={(event) =>
                        updateVariable(index, { example: event.target.value })
                      }
                    />
                    <div className="flex items-center justify-between gap-2">
                      <label className="flex items-center gap-2 text-xs text-muted-foreground">
                        <input
                          type="checkbox"
                          checked={variable.required}
                          onChange={(event) =>
                            updateVariable(index, {
                              required: event.target.checked,
                            })
                          }
                        />
                        正文必须包含该占位符
                      </label>
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        aria-label={`删除变量 ${index + 1}`}
                        onClick={() =>
                          updateDraft({
                            variables: draft.variables.filter(
                              (_, position) => position !== index
                            ),
                          })
                        }
                      >
                        <Trash2 aria-hidden="true" />
                      </Button>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <Button type="submit" size="sm" disabled={pending === "save"}>
              {pending === "save" ? (
                <LoaderCircle className="animate-spin" aria-hidden="true" />
              ) : (
                <Save aria-hidden="true" />
              )}
              {selected ? "保存修改" : "创建模板"}
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={pending === "preview" || draft.body.trim().length === 0}
              onClick={() => void handlePreview()}
            >
              {pending === "preview" ? (
                <LoaderCircle className="animate-spin" aria-hidden="true" />
              ) : (
                <Eye aria-hidden="true" />
              )}
              预览
            </Button>
            {selected ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={pending === "delete"}
                onClick={() => void handleDelete()}
              >
                {pending === "delete" ? (
                  <LoaderCircle className="animate-spin" aria-hidden="true" />
                ) : (
                  <Trash2 aria-hidden="true" />
                )}
                删除
              </Button>
            ) : null}
            {selected ? (
              <Badge variant="secondary">修订 {selected.revision}</Badge>
            ) : null}
          </div>
        </form>

        {notice ? (
          <p
            role="status"
            className={`flex items-center gap-2 text-sm ${
              notice.tone === "success"
                ? "text-muted-foreground"
                : "text-destructive"
            }`}
          >
            {notice.tone === "success" ? (
              <CircleCheck className="size-4" aria-hidden="true" />
            ) : (
              <CircleAlert className="size-4" aria-hidden="true" />
            )}
            {notice.message}
          </p>
        ) : null}

        {preview !== null ? (
          <div className="space-y-1">
            <span className="text-sm font-medium">预览（示例值）</span>
            <pre className="max-h-48 overflow-auto rounded-xl bg-muted p-3 text-xs whitespace-pre-wrap">
              {preview}
            </pre>
          </div>
        ) : null}
      </CardContent>
    </Card>
  )
}

interface TemplateInserterProps {
  /** 把渲染后的文本插入当前草稿。 */
  onInsert: (text: string) => void
  /** 用于区分同名控件的无障碍标签。 */
  label: string
  refreshToken?: number
}

/**
 * 把一个模板渲染成文本并交给调用方插入当前草稿。
 *
 * 渲染由服务端完成，因此插入的文本与保存时使用的模板规则一致。
 */
export function TemplateInserter({
  onInsert,
  label,
  refreshToken,
}: TemplateInserterProps) {
  const [templates, setTemplates] = useState<PromptTemplate[]>([])
  const [selectedId, setSelectedId] = useState("")
  const [pending, setPending] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const listed = await listPromptTemplates()
        if (!cancelled) setTemplates(listed)
      } catch {
        if (!cancelled) setTemplates([])
      }
    })()
    return () => {
      cancelled = true
    }
  }, [refreshToken])

  if (templates.length === 0) return null

  async function handleInsert() {
    const template = templates.find((item) => item.id === selectedId)
    if (!template) return
    setPending(true)
    setNotice(null)
    try {
      const response = await previewPrompt({
        kind: "template",
        body: template.body,
        variables: template.variables,
      })
      if (response.kind !== "template") {
        setNotice("插入失败：模板预览返回了意外形态")
        return
      }
      onInsert(response.rendered)
    } catch (error) {
      setNotice(errorText(error, "插入失败"))
    } finally {
      setPending(false)
    }
  }

  return (
    <div className="space-y-1">
      <div className="flex flex-wrap items-center gap-2">
        <select
          aria-label={label}
          className="h-8 min-w-32 flex-1 rounded-xl border border-input bg-background px-2 text-xs"
          value={selectedId}
          onChange={(event) => setSelectedId(event.target.value)}
        >
          <option value="">插入模板…</option>
          {templates.map((template) => (
            <option key={template.id} value={template.id}>
              {template.title}
            </option>
          ))}
        </select>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={!selectedId || pending}
          onClick={() => void handleInsert()}
        >
          {pending ? (
            <LoaderCircle className="animate-spin" aria-hidden="true" />
          ) : null}
          插入
        </Button>
      </div>
      {notice ? <p className="text-xs text-destructive">{notice}</p> : null}
    </div>
  )
}
