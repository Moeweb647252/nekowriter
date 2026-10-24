import i18n from "i18next"
import { initReactI18next } from "react-i18next"

import enAgent from "./locales/en/agent.json"
import enBook from "./locales/en/book.json"
import enChapter from "./locales/en/chapter.json"
import enCommon from "./locales/en/common.json"
import enErrors from "./locales/en/errors.json"
import enLibrary from "./locales/en/library.json"
import enPrompts from "./locales/en/prompts.json"
import enSettings from "./locales/en/settings.json"
import zhAgent from "./locales/zh-CN/agent.json"
import zhBook from "./locales/zh-CN/book.json"
import zhChapter from "./locales/zh-CN/chapter.json"
import zhCommon from "./locales/zh-CN/common.json"
import zhErrors from "./locales/zh-CN/errors.json"
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
    settings: zhSettings,
    prompts: zhPrompts,
    errors: zhErrors,
  },
} as const

/** The language to start with: the persisted choice, else the browser, else English. */
export function initialLanguage(): Language {
  const stored = localStorage.getItem("nw.language")
  if (stored === "en" || stored === "zh-CN") {
    return stored
  }
  if (navigator.language.toLowerCase().startsWith("zh")) {
    return "zh-CN"
  }
  return "en"
}

export function applyDocumentLanguage(language: Language) {
  document.documentElement.lang = language
}

i18n.use(initReactI18next).init({
  resources,
  lng: initialLanguage(),
  fallbackLng: "en",
  supportedLngs: [...SUPPORTED_LANGUAGES],
  ns: [...NAMESPACES],
  defaultNS: "common",
  interpolation: { escapeValue: false },
})

applyDocumentLanguage(i18n.language as Language)

/** Changes the language and remembers it. */
export async function setLanguage(language: Language) {
  await i18n.changeLanguage(language)
  localStorage.setItem("nw.language", language)
  applyDocumentLanguage(language)
}

export default i18n
