import type en from "../en/errors"
import type { Catalog } from "@/i18n/catalog"

export default {
  network: "无法连接 NekoWriter，请确认它仍在运行。",
  unexpected: "发生了意外情况，请重新加载页面后再试。",
  reference: "参考编号：{{requestId}}",
  field: {
    project_name: "作品名称",
    novel_title: "小说标题",
  },
  resource: {
    project: "作品",
    version: "版本",
  },
  codes: {
    FIELD_INVALID: {
      empty: "请填写{{field}}。",
      too_long: "{{field}}最多 {{maxChars}} 个字符。",
      control_character: "{{field}}只能是一行文字。",
    },
    NOT_FOUND: "该{{resource}}已不存在或已被删除。",
    REVISION_CONFLICT: "该{{resource}}已在别处被修改，现在显示的是最新内容，请再试一次。",
    IDEMPOTENCY_CONFLICT: "这个操作已经以不同的内容提交过，请重新加载页面并确认结果。",
    STATE_CONFLICT: {
      project_not_deleted: "该作品没有被删除。",
    },
    MALFORMED_REQUEST: "请求无法识别，请重新加载页面后再试。",
    ROUTE_NOT_FOUND: "当前页面与正在运行的 NekoWriter 不匹配，请重新加载页面。",
    UNTRUSTED_HOST: "NekoWriter 只响应发往本机的请求。",
    UNTRUSTED_ORIGIN: "NekoWriter 只接受来自自身页面的修改。",
    INTERNAL: "NekoWriter 遇到了意外问题，详细信息已写入日志。",
  },
} satisfies Catalog<typeof en>
