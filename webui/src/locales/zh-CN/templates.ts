import type en from "../en/templates"
import type { Catalog } from "@/i18n/catalog"

export default {
  description:
    "可复用的章节指导和意见文字。插入模板时填写其中的占位符；模板不会改变 Agent 收到的指令。",
  list: "模板",
  new: "新建模板",
  empty: "还没有模板。",
  title: "标题",
  body: "内容",
  bodyHint:
    "用双花括号包住名称来写占位符。内容中的每个占位符都要在下方列出，列出的每个占位符也都要用到。",
  variables: "占位符",
  name: "名称",
  variableDescription: "填写说明",
  addVariable: "添加占位符",
  removeVariable: "移除占位符 {{name}}",
  create: "创建模板",
  save: "保存模板",
  saved: "已保存",
  delete: "删除",
  deleteTitle: "删除“{{title}}”？",
  deleteBody: "模板将被永久删除。已经插入的文字不受影响。",
  deleteConfirm: "删除模板",
  insert: {
    open: "插入模板",
    title: "插入模板",
    choose: "模板",
    none: "还没有模板。请先在提示词页面中创建。",
    submit: "插入",
  },
} satisfies Catalog<typeof en>
