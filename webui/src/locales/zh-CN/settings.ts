import type en from "../en/settings"
import type { Catalog } from "@/i18n/catalog"

export default {
  title: "设置",
  appearance: {
    title: "外观",
    language: "界面语言",
    languageHint: "菜单和提示信息使用的语言。每个作品另有自己的写作语言。",
    theme: "主题",
    themes: {
      system: "跟随系统",
      light: "浅色",
      dark: "深色",
    },
  },
  diagnostics: {
    title: "本机安装",
    version: "版本",
    dataDir: "数据目录",
    logDir: "日志目录",
  },
  service: {
    title: "本地服务",
    body: "关闭本页后 NekoWriter 仍会在后台运行。用完后可以在这里退出。",
    exit: "退出 NekoWriter",
    confirmTitle: "退出 NekoWriter？",
    confirmBody: "正在进行的写作会停止，下次启动后可以继续。所有数据都保留在数据目录中。",
    confirm: "退出",
  },
} satisfies Catalog<typeof en>
