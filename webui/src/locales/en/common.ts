import type { ProjectListFilter } from "@/api/generated/ProjectListFilter"
import type { WritingLanguage } from "@/api/generated/WritingLanguage"

export default {
  appName: "NekoWriter",
  nav: {
    label: "Main",
    works: "Works",
    dashboard: "Dashboard",
    prompts: "Prompts",
    knowledge: "Knowledge",
    settings: "Settings",
  },
  actions: {
    cancel: "Cancel",
    close: "Close",
    retry: "Try again",
    reload: "Reload",
  },
  status: {
    loading: "Loading…",
    saving: "Saving…",
  },
  writingLanguage: {
    "zh-CN": "Simplified Chinese",
    en: "English",
  } satisfies Record<WritingLanguage, string>,
  projectFilter: {
    active: "In progress",
    deleted: "Deleted",
  } satisfies Record<ProjectListFilter, string>,
  connection: {
    reconnecting: "Reconnecting to NekoWriter…",
    stoppedTitle: "NekoWriter has stopped",
    stoppedBody: "Start NekoWriter again, then reload this page. Your work is saved.",
  },
} as const
