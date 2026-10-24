import "@testing-library/jest-dom/vitest"

import i18n from "i18next"
import { initReactI18next } from "react-i18next"

import enCommon from "@/i18n/locales/en/common.json"
import enLibrary from "@/i18n/locales/en/library.json"

// Tests run in English with the real resource files so missing keys fail loudly.
if (!i18n.isInitialized) {
  void i18n.use(initReactI18next).init({
    resources: { en: { common: enCommon, library: enLibrary } },
    lng: "en",
    fallbackLng: "en",
    ns: ["common", "library"],
    defaultNS: "common",
    interpolation: { escapeValue: false },
  })
}
