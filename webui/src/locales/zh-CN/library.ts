import type en from "../en/library"
import type { Catalog } from "@/i18n/catalog"

export default {
  title: "作品",
  newWork: "新建作品",
  filterLabel: "显示",
  empty: {
    activeTitle: "还没有作品",
    activeBody: "新建一个作品，开始写小说。",
    deletedTitle: "没有已删除的作品",
    deletedBody: "删除的作品会出现在这里，可以随时恢复。",
  },
  card: {
    version: "第 {{number}} 版",
    updated: "{{time}}更新",
    deleted: "{{time}}删除",
    actions: "{{name}}的操作",
    rename: "重命名",
    delete: "删除",
    restore: "恢复",
  },
  create: {
    title: "新建作品",
    description: "一个作品包含一部小说。除小说本身外，其余内容之后都可以修改。",
    name: "作品名称",
    novelTitle: "小说标题",
    writingLanguage: "写作语言",
    writingLanguageHint: "Agent 会用这种语言撰写小说。",
    submit: "创建作品",
  },
  rename: {
    title: "重命名作品",
    name: "作品名称",
    submit: "重命名",
  },
  delete: {
    title: "删除“{{name}}”？",
    body: "作品会移到“已删除”，之后可以恢复。",
    confirm: "删除作品",
  },
  done: {
    created: "已创建“{{name}}”。",
    renamed: "已重命名为“{{name}}”。",
    deleted: "已删除“{{name}}”。",
    restored: "已恢复“{{name}}”。",
  },
} satisfies Catalog<typeof en>
