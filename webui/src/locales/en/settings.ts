import type { Theme } from "@/app/theme"

export default {
  title: "Settings",
  appearance: {
    title: "Appearance",
    language: "Interface language",
    languageHint: "The language of menus and messages. Each work has its own writing language.",
    theme: "Theme",
    themes: {
      system: "Match the system",
      light: "Light",
      dark: "Dark",
    } satisfies Record<Theme, string>,
  },
  diagnostics: {
    title: "This installation",
    version: "Version",
    dataDir: "Data directory",
    logDir: "Log directory",
  },
  service: {
    title: "Local service",
    body: "NekoWriter keeps running after you close this page. Exit it here when you are done.",
    exit: "Exit NekoWriter",
    confirmTitle: "Exit NekoWriter?",
    confirmBody:
      "Work in progress stops and can be continued after the next start. All data stays in the data directory.",
    confirm: "Exit",
  },
} as const
