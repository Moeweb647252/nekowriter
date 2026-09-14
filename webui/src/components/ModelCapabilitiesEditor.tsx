import { Cable, CircleAlert } from "lucide-react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from "@/components/ui/field"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import type { ModelCapabilityProfile } from "@/lib/api"

/** 「支持思考」这一声明的验证状态。 */
export interface ThinkingVerification {
  /**
   * 当前模型 ID 是否已经通过一次真的携带 `reasoning_effort` 的连接测试。
   *
   * 未验证时不允许新声明支持思考：否则界面上显示的「支持思考」只是用户的猜测，
   * 而调度器会按这个声明放行 `reasoning_effort` 请求。
   */
  verified: boolean
  verifying: boolean
  canVerify: boolean
  onVerify: () => void
  /** 不能立即验证时说明原因。 */
  blockedReason: string | null
}

interface ModelCapabilitiesEditorProps {
  /** 同一页面可能出现多个编辑器，用它派生不重复的表单控件 id。 */
  idPrefix: string
  value: ModelCapabilityProfile
  onChange: (next: ModelCapabilityProfile) => void
  disabled?: boolean
  thinking: ThinkingVerification
}

/**
 * 模型能力声明编辑器。
 *
 * 能力资料描述的是「某个具体模型」，因此系统写作模型与项目模型引用共用这一个编辑器，
 * 避免两处出现不同的字段集合或不同的验证规则。
 */
export function ModelCapabilitiesEditor({
  idPrefix,
  value,
  onChange,
  disabled = false,
  thinking,
}: ModelCapabilitiesEditorProps) {
  const locked = disabled

  function update<K extends keyof ModelCapabilityProfile>(
    key: K,
    next: ModelCapabilityProfile[K]
  ) {
    onChange({ ...value, [key]: next })
  }

  return (
    <FieldSet>
      <FieldLegend>模型能力</FieldLegend>
      <FieldDescription>
        写作流程会在发出请求前验证这些能力，避免把不兼容的配置当作可用服务。
      </FieldDescription>
      <Field orientation="horizontal" data-disabled={locked || undefined}>
        <Switch id={`${idPrefix}-streaming`} checked disabled />
        <FieldContent>
          <FieldLabel htmlFor={`${idPrefix}-streaming`}>流式响应</FieldLabel>
          <FieldDescription>写作请求必须支持流式输出。</FieldDescription>
        </FieldContent>
      </Field>
      <Field orientation="horizontal" data-disabled={locked || undefined}>
        <Switch
          id={`${idPrefix}-tools`}
          checked={value.tools}
          onCheckedChange={(tools) => update("tools", tools)}
          disabled={locked}
        />
        <FieldContent>
          <FieldLabel htmlFor={`${idPrefix}-tools`}>工具调用</FieldLabel>
          <FieldDescription>允许模型调用受控的写作工具。</FieldDescription>
        </FieldContent>
      </Field>
      <Field orientation="horizontal" data-disabled={locked || undefined}>
        <Switch
          id={`${idPrefix}-stream-usage`}
          checked={value.streamUsage}
          onCheckedChange={(streamUsage) => update("streamUsage", streamUsage)}
          disabled={locked}
        />
        <FieldContent>
          <FieldLabel htmlFor={`${idPrefix}-stream-usage`}>流式用量</FieldLabel>
          <FieldDescription>服务可在流结束时返回 Token 用量。</FieldDescription>
        </FieldContent>
      </Field>
      <Field orientation="horizontal" data-disabled={locked || undefined}>
        <Switch
          id={`${idPrefix}-thinking`}
          checked={value.thinking}
          onCheckedChange={(next) => update("thinking", next)}
          disabled={locked || !thinking.verified}
        />
        <FieldContent>
          <FieldLabel htmlFor={`${idPrefix}-thinking`}>支持思考</FieldLabel>
          <FieldDescription>
            {thinking.verified
              ? "已通过携带 reasoning_effort 的连接测试确认。"
              : "只有验证通过后才能声明；未验证的声明会让下一次思考请求在服务端失败。"}
          </FieldDescription>
        </FieldContent>
      </Field>

      {!thinking.verified && (
        <div className="flex flex-wrap items-center gap-3">
          <Button
            type="button"
            variant="outline"
            onClick={thinking.onVerify}
            disabled={locked || thinking.verifying || !thinking.canVerify}
          >
            {thinking.verifying ? (
              <Spinner data-icon="inline-start" aria-hidden="true" />
            ) : (
              <Cable data-icon="inline-start" aria-hidden="true" />
            )}
            {thinking.verifying ? "正在验证…" : "验证思考支持"}
          </Button>
          {thinking.blockedReason && (
            <span className="text-xs text-muted-foreground">
              {thinking.blockedReason}
            </span>
          )}
        </div>
      )}

      {!value.tools && (
        <Alert>
          <CircleAlert aria-hidden="true" />
          <AlertTitle>写作阶段需要工具调用</AlertTitle>
          <AlertDescription>
            当前声明不支持工具调用，写作请求会在发出前被拒绝；只有审核阶段可以这样配置。
          </AlertDescription>
        </Alert>
      )}
    </FieldSet>
  )
}
