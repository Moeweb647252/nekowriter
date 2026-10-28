import "@testing-library/jest-dom/vitest"

import i18n from "i18next"
import { initReactI18next } from "react-i18next"

import enAgent from "@/i18n/locales/en/agent.json"
import enBook from "@/i18n/locales/en/book.json"
import enChapter from "@/i18n/locales/en/chapter.json"
import enCommon from "@/i18n/locales/en/common.json"
import enErrors from "@/i18n/locales/en/errors.json"
import enLibrary from "@/i18n/locales/en/library.json"
import enSkills from "@/i18n/locales/en/skills.json"

// jsdom implements no ResizeObserver, and the Radix popper/size primitives that shadcn's `Tooltip`
// and `DropdownMenu` are built on construct one as soon as their content mounts. Without a stub the
// throw happens in a layout effect after the test's own assertions, so the suite fails as an
// "unhandled error" attributed to whichever test file happened to mount a tooltip — which made
// `conceptionStudio.test.tsx` red in roughly one full run out of three with no defect behind it.
// A no-op observer is enough here: no test asserts on measured sizes, only on rendered output, and
// browsers provide the real one.
if (!("ResizeObserver" in globalThis)) {
  class TestResizeObserver implements ResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  globalThis.ResizeObserver = TestResizeObserver
}

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
        skills: enSkills,
      },
    },
    lng: "en",
    fallbackLng: "en",
    ns: ["agent", "book", "chapter", "common", "errors", "library", "skills"],
    defaultNS: "common",
    interpolation: { escapeValue: false },
  })
}
