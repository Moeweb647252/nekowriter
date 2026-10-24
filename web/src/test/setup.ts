import "@testing-library/jest-dom/vitest"

import i18n from "i18next"
import { initReactI18next } from "react-i18next"

import enAgent from "@/i18n/locales/en/agent.json"
import enBook from "@/i18n/locales/en/book.json"
import enChapter from "@/i18n/locales/en/chapter.json"
import enCommon from "@/i18n/locales/en/common.json"
import enErrors from "@/i18n/locales/en/errors.json"
import enLibrary from "@/i18n/locales/en/library.json"

// Tests run in English with the real resource files so missing keys fail loudly. Every namespace
// a component under test may use has to be here, or `t()` silently renders the bare key.
if (!i18n.isInitialized) {
  void i18n.use(initReactI18next).init({
    resources: {
      en: {
        agent: enAgent,
        book: enBook,
        chapter: enChapter,
        common: enCommon,
        errors: enErrors,
        library: enLibrary,
      },
    },
    lng: "en",
    fallbackLng: "en",
    ns: ["agent", "book", "chapter", "common", "errors", "library"],
    defaultNS: "common",
    interpolation: { escapeValue: false },
  })
}
