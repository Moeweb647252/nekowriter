/** Every generated error code has an `errors:` key in both locales (spec/11 §9).
 *
 * `check-i18n` can only see literal keys, so the codes that arrive as data are asserted here. */
import { describe, expect, it } from "vitest"

import type { ErrorCode } from "@/api/gen/ErrorCode"
import { NAMESPACES, SUPPORTED_LANGUAGES, resources } from "@/i18n"
import enErrors from "@/i18n/locales/en/errors.json"
import zhErrors from "@/i18n/locales/zh-CN/errors.json"
import enChapter from "@/i18n/locales/en/chapter.json"
import zhChapter from "@/i18n/locales/zh-CN/chapter.json"

/** The values of a generated string-union type, read from the `ErrorCode.ts` module. */
const ALL_ERROR_CODES: ErrorCode[] = [
  "validation.failed",
  "not_found",
  "host.forbidden",
  "conflict.version",
  "conflict.exists",
  "book.invalid_state",
  "research.invalid_state",
  "settings.locked",
  "chapter.invalid_state",
  "chapter.session_exists",
  "chapter.previous_not_ready",
  "chapter.is_anchor",
  "chapter.locked",
  "session.invalid_state",
  "model.missing_role",
  "conception.incomplete",
  "track.mode_locked",
  "change.not_latest",
  "proposal.invalid_state",
  "prompt.template_invalid",
  "provider.unavailable",
  "provider.auth",
  "provider.bad_request",
  "embedding.unavailable",
  "search.not_configured",
  "internal",
]

const CHAPTER_STATUSES = ["planned", "drafting", "in_review", "revising", "official"]
const CANON_STATUSES = ["none", "pending", "syncing", "synced", "stale", "failed"]

describe("locale coverage", () => {
  it("has an errors key for every ErrorCode", () => {
    const en = enErrors as Record<string, string>
    const zh = zhErrors as Record<string, string>
    for (const code of ALL_ERROR_CODES) {
      expect(en[code], `en errors:${code}`).toBeTypeOf("string")
      expect(zh[code], `zh-CN errors:${code}`).toBeTypeOf("string")
    }
  })

  it("has a label for every chapter and canon status StatusBadge can render", () => {
    for (const [locale, table] of [
      ["en", enChapter as Record<string, string>],
      ["zh-CN", zhChapter as Record<string, string>],
    ] as const) {
      for (const status of CHAPTER_STATUSES) {
        expect(table[`status.${status}`], `${locale} chapter:status.${status}`).toBeTypeOf(
          "string",
        )
      }
      for (const status of CANON_STATUSES) {
        expect(table[`canon_status.${status}`], `${locale} chapter:canon_status.${status}`).toBeTypeOf(
          "string",
        )
      }
      for (const granularity of ["sentence", "paragraph", "free", "chapter"]) {
        expect(
          table[`granularity.${granularity}`],
          `${locale} chapter:granularity.${granularity}`,
        ).toBeTypeOf("string")
      }
      for (const kind of ["draft", "revision", "official", "manual", "backup"]) {
        expect(
          table[`version_kind.${kind}`],
          `${locale} chapter:version_kind.${kind}`,
        ).toBeTypeOf("string")
      }
    }
  })
})

/** PLAN 0.5 requires every namespace from spec/11 §2 to exist in both locales. */
describe("i18n namespaces", () => {
  const SPEC_NAMESPACES = [
    "common",
    "library",
    "book",
    "chapter",
    "agent",
    "canon",
    "knowledge",
    "prompts",
    "settings",
    "errors",
  ]

  it("ships every namespace spec/11 §2 names, in both locales", () => {
    expect([...NAMESPACES].sort()).toEqual([...SPEC_NAMESPACES].sort())
    for (const language of SUPPORTED_LANGUAGES) {
      for (const namespace of SPEC_NAMESPACES) {
        const table = (resources[language] as Record<string, unknown>)[namespace]
        expect(table, `${language}/${namespace}.json is not bundled`).toBeTypeOf("object")
        expect(Object.keys(table as object).length).toBeGreaterThan(0)
      }
    }
  })
})
