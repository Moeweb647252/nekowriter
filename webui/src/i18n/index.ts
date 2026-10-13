import i18n from "i18next"
import LanguageDetector from "i18next-browser-languagedetector"
import { initReactI18next } from "react-i18next"

import { NAMESPACES, en } from "@/locales/en"
import { UI_LANGUAGES, uiLanguageFor } from "@/locales/languages"
import { zhCN } from "@/locales/zh-CN"

/** Where the chosen interface language is remembered in this browser. */
export const LANGUAGE_STORAGE_KEY = "nekowriter.uiLanguage"

export const resources = { en, "zh-CN": zhCN }

void i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources,
    supportedLngs: [...UI_LANGUAGES],
    // Every catalog has every key (enforced by the type checker), so no other language is
    // ever needed to fill gaps.
    fallbackLng: false,
    ns: NAMESPACES,
    defaultNS: NAMESPACES,
    // Must match `enableSelector` in `i18next.d.ts`: selectors start with their namespace.
    enableSelector: "strict",
    initAsync: false,
    interpolation: { escapeValue: false },
    detection: {
      order: ["localStorage", "navigator"],
      lookupLocalStorage: LANGUAGE_STORAGE_KEY,
      caches: ["localStorage"],
      convertDetectedLanguage: uiLanguageFor,
    },
  })

function syncDocumentLanguage(language: string) {
  document.documentElement.lang = language
}

syncDocumentLanguage(i18n.resolvedLanguage ?? i18n.language)
i18n.on("languageChanged", syncDocumentLanguage)

export default i18n
