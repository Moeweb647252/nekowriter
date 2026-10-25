/** Every generated error code has an `errors:` key in both locales (spec/11 §9).
 *
 * `check-i18n` can only see literal keys, so the codes that arrive as data are asserted here. */
import { describe, expect, it } from "vitest"
import i18n from "i18next"

import type { ErrorCode } from "@/api/gen/ErrorCode"
import { NAMESPACES, SUPPORTED_LANGUAGES, resources } from "@/i18n"
import enErrors from "@/i18n/locales/en/errors.json"
import zhErrors from "@/i18n/locales/zh-CN/errors.json"
import enChapter from "@/i18n/locales/en/chapter.json"
import zhChapter from "@/i18n/locales/zh-CN/chapter.json"
import enAgent from "@/i18n/locales/en/agent.json"
import zhAgent from "@/i18n/locales/zh-CN/agent.json"
import { registeredTools, toolRenderer } from "@/features/agent/toolRenderers"

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

/**
 * Warning codes the API may return in `warnings` (spec/10 §1). They arrive as data, so
 * `check-i18n` cannot see them and they are asserted here (spec/11 §9).
 */
const ALL_WARNING_CODES = [
  "model.missing_role",
  "context.large_kickoff",
  "prompt.override_failed",
  "conception.unavailable",
]

/**
 * Session error codes: `SessionDto.error.code` (spec/10 §4's second list), not HTTP codes. Four of
 * them overlap `ErrorCode` and are covered above; the rest reach the UI only through a session
 * row, so they need the same `errors:` keys and the same assertion.
 */
const ALL_SESSION_ERROR_CODES = [
  "provider.unavailable",
  "provider.auth",
  "provider.bad_request",
  "provider.context_overflow",
  "provider.refusal",
  "agent.tool_error_streak",
  "agent.no_tool_call",
  "model.missing_provider",
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

  it("has an errors key for every warning code", () => {
    const en = enErrors as Record<string, string>
    const zh = zhErrors as Record<string, string>
    for (const code of ALL_WARNING_CODES) {
      expect(en[code], `en errors:${code}`).toBeTypeOf("string")
      expect(zh[code], `zh-CN errors:${code}`).toBeTypeOf("string")
    }
  })

  it("localizes every warning code through the real i18n instance", () => {
    // The toast helper resolves codes with t("errors:<code>"); a missing key would render the raw
    // code at the author, so assert the resolution, not just the JSON tables.
    for (const language of SUPPORTED_LANGUAGES) {
      for (const code of ALL_WARNING_CODES) {
        const text = i18n.getFixedT(language)?.("errors:" + code, { defaultValue: code })
        expect(text, `${language} errors:${code}`).not.toBe(code)
        expect(text, `${language} errors:${code}`).toBeTruthy()
      }
    }
  })

  it("has an errors key for every session error code", () => {
    // spec/11 §9: every session error code needs an `errors:` key in both locales. They are
    // stored as data in `agent_sessions.error` and rendered by `SessionErrorNotice`, so
    // `check-i18n` cannot see them.
    const en = enErrors as Record<string, string>
    const zh = zhErrors as Record<string, string>
    for (const code of ALL_SESSION_ERROR_CODES) {
      expect(en[code], `en errors:${code}`).toBeTypeOf("string")
      expect(zh[code], `zh-CN errors:${code}`).toBeTypeOf("string")
    }
    for (const language of SUPPORTED_LANGUAGES) {
      for (const code of ALL_SESSION_ERROR_CODES) {
        const text = i18n.getFixedT(language)?.("errors:" + code, { defaultValue: code })
        expect(text, `${language} errors:${code}`).not.toBe(code)
      }
    }
  })

  it("has a summary key for every tool in spec/05 §2, in both locales", () => {
    // spec/11 §5.3: "Each tool has a summary i18n key `agent:tools.<name>.summary`". The key is
    // built from the tool name at runtime, so `check-i18n` cannot see it and this is the only
    // place a missing one is caught (it caught `request_review`, whose renderer returned "").
    const specTools = [
      // writer
      "get_entity",
      "list_entities",
      "get_relations",
      "query_timeline",
      "list_foreshadowing",
      "search_canon",
      "search_knowledge",
      "read_knowledge_entry",
      "search_chapters",
      "read_chapter",
      "submit_outline",
      "submit_revision_plan",
      "read_draft",
      "insert_text",
      "insert_paragraphs",
      "replace_text",
      "rewrite_paragraph",
      "delete_paragraphs",
      "finish_draft",
      "finish_revision",
      // reviewer
      "submit_review",
      // archivist
      "create_entity",
      "update_entity",
      "set_relation",
      "end_relation",
      "record_appearances",
      "add_timeline_event",
      "update_timeline_event",
      "add_memory_note",
      "retire_memory_note",
      "add_foreshadowing",
      "resolve_foreshadowing",
      "set_chapter_summary",
      "propose_setting_change",
      "finish_maintenance",
      // conception
      "ask_user",
      "web_search",
      "web_fetch",
      "write_setting_doc",
      "edit_setting_doc",
      "upsert_character",
      "remove_character",
      "reorder_characters",
      "set_research_topics",
      "request_review",
      // researcher
      "save_knowledge_entry",
      "update_knowledge_entry",
      "finish_research",
    ]
    // `submit_review` has one key per verdict (`summary_pass` / `summary_revise`) because the
    // renderer must choose between them from `ui.verdict` (spec/11 §5.3 allows either shape:
    // the key is `<name>.summary`, and a variant suffix is still that family).
    const variantKeys: Record<string, string[]> = {
      submit_review: ["summary_pass", "summary_revise"],
    }
    for (const [locale, table] of [
      ["en", enAgent as Record<string, unknown>],
      ["zh-CN", zhAgent as Record<string, unknown>],
    ] as const) {
      const tools = (table.tools ?? {}) as Record<string, Record<string, string>>
      for (const name of specTools) {
        const entry = tools[name] ?? {}
        const keys = variantKeys[name] ?? ["summary"]
        for (const key of keys) {
          expect(entry[key], `${locale} agent:tools.${name}.${key}`).toBeTypeOf("string")
          expect(entry[key]?.length ?? 0, `${locale} agent:tools.${name}.${key}`).toBeGreaterThan(
            0,
          )
        }
      }
    }
    // Prove it through the real renderer and the real i18n instance rather than assuming the key
    // shape: a renderer that returns "" (which is how `request_review` hid its missing key) or a
    // key that does not resolve both surface here. The debug session's test-only tools and `echo`
    // are excluded: they never reach a real session of a role list (spec/10 §2.7).
    const debugTools = new Set(["echo", "test_write", "test_finish", "test_slow"])
    for (const name of registeredTools()) {
      if (debugTools.has(name)) {
        continue
      }
      for (const language of SUPPORTED_LANGUAGES) {
        const t = i18n.getFixedT(language)
        // An empty `ui` payload exercises the renderer's no-data branch, which must still be a
        // translated string and not the raw key or an empty summary.
        const summary = toolRenderer(name).summary({}, t)
        expect(summary, `${language} agent:tools.${name}`).not.toBe("")
        expect(summary, `${language} agent:tools.${name}`).not.toContain("agent:tools.")
      }
    }
  })

  it("has a session title key for every SessionKind", () => {
    // spec/11 §9: every SessionKind needs `agent:session_title.<kind>`. The key is composed at
    // runtime, so it is asserted here.
    const kinds = [
      "conception",
      "research",
      "chapter_write",
      "chapter_revise",
      "chapter_review",
      "chapter_maintenance",
      "canon_seed",
      "chapter_resync",
    ]
    for (const [locale, table] of [
      ["en", enAgent as Record<string, unknown>],
      ["zh-CN", zhAgent as Record<string, unknown>],
    ] as const) {
      const titles = (table.session_title ?? {}) as Record<string, string>
      for (const kind of kinds) {
        expect(titles[kind], `${locale} agent:session_title.${kind}`).toBeTypeOf("string")
      }
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
