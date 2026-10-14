import type en from "../en/common"
import type { Catalog } from "@/i18n/catalog"

export default {
  appName: "NekoWriter",
  nav: {
    label: "主导航",
    works: "作品",
    dashboard: "仪表盘",
    settings: "设置",
  },
  actions: {
    cancel: "取消",
    close: "关闭",
    retry: "重试",
    reload: "重新加载",
  },
  status: {
    loading: "正在加载…",
    saving: "正在保存…",
  },
  writingLanguage: {
    "zh-CN": "简体中文",
    en: "英语",
  },
  projectFilter: {
    active: "进行中",
    deleted: "已删除",
  },
  connection: {
    reconnecting: "正在重新连接 NekoWriter…",
    stoppedTitle: "NekoWriter 已退出",
    stoppedBody: "请重新启动 NekoWriter，然后重新加载本页。你的作品都已保存。",
  },
} satisfies Catalog<typeof en>
