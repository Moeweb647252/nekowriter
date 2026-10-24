import i18n from "i18next"
import { initReactI18next } from "react-i18next"

import { useUiStore } from "@/stores/ui"

import enAgent from "./locales/en/agent.json"
import enBook from "./locales/en/book.json"
import enCanon from "./locales/en/canon.json"
import enChapter from "./locales/en/chapter.json"
import enCommon from "./locales/en/common.json"
import enErrors from "./locales/en/errors.json"
import enKnowledge from "./locales/en/knowledge.json"
import enLibrary from "./locales/en/library.json"
import enPrompts from "./locales/en/prompts.json"
import enSettings from "./locales/en/settings.json"
import zhAgent from "./locales/zh-CN/agent.json"
import zhBook from "./locales/zh-CN/book.json"
import zhCanon from "./locales/zh-CN/canon.json"
import zhChapter from "./locales/zh-CN/chapter.json"
import zhCommon from "./locales/zh-CN/common.json"
import zhErrors from "./locales/zh-CN/errors.json"
import zhKnowledge from "./locales/zh-CN/knowledge.json"
import zhLibrary from "./locales/zh-CN/library.json"
import zhPrompts from "./locales/zh-CN/prompts.json"
import zhSettings from "./locales/zh-CN/settings.json"

export const SUPPORTED_LANGUAGES = ["en", "zh-CN"] as const
export type Language = (typeof SUPPORTED_LANGUAGES)[number]

/** Namespaces shipped with the app; every namespace exists in both locales. */
export const NAMESPACES = [
  "common",
  "library",
  "book",
  "chapter",
  "agent",
  "canon",
  "knowledge",
  "settings",
  "prompts",
  "errors",
] as const

/** Every bundled resource, imported statically (spec/11 §6). */
export const resources = {
  en: {
    common: enCommon,
    library: enLibrary,
    book: enBook,
    chapter: enChapter,
    agent: enAgent,
    canon: enCanon,
    knowledge: enKnowledge,
    settings: enSettings,
    prompts: enPrompts,
    errors: enErrors,
  },
  "zh-CN": {
    common: zhCommon,
    library: zhLibrary,
    book: zhBook,
    chapter: zhChapter,
    agent: zhAgent,
    canon: zhCanon,
    knowledge: zhKnowledge,
    settings: zhSettings,
    prompts: zhPrompts,
    errors: zhErrors,
  },
} as const

/**
 * The language to start with: the persisted choice, else the browser, else English
 * (spec/11 §6). The choice itself lives in the `ui` store (spec/11 §4.3), which is what persists
 * it; this only decides the value on a first run.
 */
export function initialLanguage(): Language {
  if (typeof navigator !== "undefined" && navigator.language.toLowerCase().startsWith("zh")) {
    return "zh-CN"
  }
  return "en"
}

export function applyDocumentLanguage(language: Language) {
  document.documentElement.lang = language
}

/**
 * The persisted choice from the `ui` store, else the browser language, else English
 * (spec/11 §6). The store rehydrates synchronously from localStorage before this runs, because
 * `i18n` imports it and the module is evaluated during startup.
 */
export function resolveLanguage(): Language {
  const stored = useUiStore.getState().language
  if (stored === "en" || stored === "zh-CN") {
    return stored
  }
  return initialLanguage()
}

i18n.use(initReactI18next).init({
  resources,
  lng: resolveLanguage(),
  fallbackLng: "en",
  supportedLngs: [...SUPPORTED_LANGUAGES],
  ns: [...NAMESPACES],
  defaultNS: "common",
  interpolation: { escapeValue: false },
})

applyDocumentLanguage(i18n.language as Language)

/**
 * Changes the language and remembers it. `ui.language` is the persisted value (spec/11 §4.3),
 * so this writes it to the store as well as switching i18next (spec/11 §6).
 */
export async function setLanguage(language: Language) {
  await i18n.changeLanguage(language)
  applyDocumentLanguage(language)
  useUiStore.getState().setLanguage(language)
}

export default i18n
