/**
 * The tool renderer registry (spec/11 §5.3): every tool gets an icon, a localized one-line
 * summary built from its `ui` payload, and optionally a detail component.
 *
 * A `ui` payload carries data only (ids, numbers, enum values, stored names), never labels
 * rendered by the prompt pack, so the summary is composed here through i18n (D-054).
 * Unknown tools fall back to the tool name plus the raw JSON details.
 */
import type { LucideIcon } from "lucide-react"
import {
  Archive,
  BookOpen,
  Bot,
  ClipboardCheck,
  Compass,
  Download,
  FileText,
  List,
  PenLine,
  Search,
  Sparkles,
  Target,
  Users,
  Wrench,
} from "lucide-react"

import type { TFunction } from "i18next"

/** The value shape of a `ui` payload (an opaque JSON object from the API). */
type UiPayload = Record<string, unknown>

export type ToolRenderer = {
  icon: LucideIcon
  /** The localized one-line summary (PROJECT §8.6 examples). */
  summary: (ui: unknown, t: TFunction) => string
}

/** Reads a number-ish field of a `ui` payload, or `undefined`. */
function num(ui: UiPayload, key: string): number | undefined {
  const value = ui[key]
  return typeof value === "number" ? value : undefined
}

/** Reads a string field of a `ui` payload, or `undefined`. */
function str(ui: UiPayload, key: string): string | undefined {
  const value = ui[key]
  return typeof value === "string" ? value : undefined
}

/** Reads a string array of a `ui` payload. */
function strArray(ui: UiPayload, key: string): string[] {
  const value = ui[key]
  if (!Array.isArray(value)) {
    return []
  }
  return value.filter((item): item is string => typeof item === "string")
}

function payload(ui: unknown): UiPayload {
  return typeof ui === "object" && ui !== null ? (ui as UiPayload) : {}
}

/**
 * The i18n key of a tool's summary (`tools.<name>.summary`).
 *
 * The key is namespace-relative: every call site passes it through the renderer's `t`, which is a
 * component `t()` with `common` as its default namespace, so the lookup needs the explicit
 * `agent:` prefix. Exported without it for the callers that build a sibling key from it, so
 * [`summaryKey`] and its `_target` sibling stay in one place.
 */
export function summaryKey(name: string): string {
  return `tools.${name}.summary`
}

/** The namespace-qualified key a summary is looked up with. */
function summaryI18nKey(name: string): string {
  return `agent:${summaryKey(name)}`
}

const DEFAULT: ToolRenderer = {
  icon: Wrench,
  summary: (ui, t) => {
    const keys = Object.keys(payload(ui))
    return keys.length === 0 ? t("agent:tools.unknown") : t("agent:tools.unknown_with", { count: keys.length })
  },
}

const REGISTRY: Record<string, ToolRenderer> = {
  // Retrieval
  get_entity: {
    icon: Users,
    summary: (ui, t) => {
      const name = str(payload(ui), "name")
      return name === undefined
        ? t("agent:tools.get_entity.summary")
        : t("agent:tools.get_entity.summary_named", { name })
    },
  },
  list_entities: {
    icon: List,
    summary: (ui, t) => t("agent:tools.list_entities.summary", { count: num(payload(ui), "count") ?? 0 }),
  },
  get_relations: {
    icon: Users,
    summary: (ui, t) => t("agent:tools.get_relations.summary", { count: num(payload(ui), "count") ?? 0 }),
  },
  query_timeline: {
    icon: Compass,
    summary: (ui, t) => t("agent:tools.query_timeline.summary", { count: num(payload(ui), "count") ?? 0 }),
  },
  list_foreshadowing: {
    icon: List,
    summary: (ui, t) =>
      t("agent:tools.list_foreshadowing.summary", { count: num(payload(ui), "count") ?? 0 }),
  },
  search_canon: {
    icon: Search,
    summary: (ui, t) => {
      const data = payload(ui)
      const query = str(data, "query")
      return query === undefined
        ? t("agent:tools.search_canon.summary", { count: num(data, "count") ?? 0 })
        : t("agent:tools.search_canon.summary_query", {
            query,
            count: num(data, "count") ?? 0,
          })
    },
  },
  search_knowledge: {
    icon: Search,
    summary: (ui, t) => {
      const data = payload(ui)
      const query = str(data, "query")
      return query === undefined
        ? t("agent:tools.search_knowledge.summary", { count: num(data, "count") ?? 0 })
        : t("agent:tools.search_knowledge.summary_query", {
            query,
            count: num(data, "count") ?? 0,
          })
    },
  },
  read_knowledge_entry: {
    icon: BookOpen,
    summary: (ui, t) => {
      const title = str(payload(ui), "title")
      return title === undefined
        ? t("agent:tools.read_knowledge_entry.summary")
        : t("agent:tools.read_knowledge_entry.summary_title", { title })
    },
  },
  search_chapters: {
    icon: Search,
    summary: (ui, t) =>
      t("agent:tools.search_chapters.summary", { count: num(payload(ui), "count") ?? 0 }),
  },
  read_chapter: {
    icon: BookOpen,
    summary: (ui, t) => {
      const data = payload(ui)
      const number = num(data, "number")
      return number === undefined
        ? t("agent:tools.read_chapter.summary")
        : t("agent:tools.read_chapter.summary_chapter", { number })
    },
  },

  // Writer
  submit_outline: {
    icon: FileText,
    summary: (ui, t) => {
      const data = payload(ui)
      return t("agent:tools.submit_outline.summary", {
        outline: num(data, "outline_chars") ?? 0,
        notes: num(data, "notes_chars") ?? 0,
      })
    },
  },
  submit_revision_plan: {
    icon: FileText,
    summary: (ui, t) =>
      t("agent:tools.submit_revision_plan.summary", { chars: num(payload(ui), "plan_chars") ?? 0 }),
  },
  read_draft: {
    icon: FileText,
    summary: (ui, t) => t("agent:tools.read_draft.summary", { count: num(payload(ui), "paragraphs") ?? 0 }),
  },
  insert_text: {
    icon: PenLine,
    summary: (ui, t) => {
      const data = payload(ui)
      const inserted = strArray(data, "inserted")
      const appendedTo = str(data, "appended_to")
      const tailOpen = data["tail_open"] === true
      if (appendedTo !== undefined) {
        return tailOpen
          ? t("agent:tools.insert_text.summary_appended_open", { pid: appendedTo })
          : t("agent:tools.insert_text.summary_appended", { pid: appendedTo })
      }
      if (inserted.length === 0) {
        return t("agent:tools.insert_text.summary")
      }
      return tailOpen
        ? t("agent:tools.insert_text.summary_inserted_open", { pids: inserted.join(", ") })
        : t("agent:tools.insert_text.summary_inserted", { pids: inserted.join(", ") })
    },
  },
  insert_paragraphs: {
    icon: PenLine,
    summary: (ui, t) => {
      const data = payload(ui)
      const inserted = strArray(data, "inserted")
      return t("agent:tools.insert_paragraphs.summary", { pids: inserted.join(", ") })
    },
  },
  replace_text: {
    icon: PenLine,
    summary: (ui, t) => {
      const data = payload(ui)
      const pid = str(data, "pid")
      const after = data["after"]
      if (pid === undefined) {
        return t("agent:tools.replace_text.summary")
      }
      return after === "" || after === null
        ? t("agent:tools.replace_text.summary_deleted", { pid })
        : t("agent:tools.replace_text.summary", { pid })
    },
  },
  rewrite_paragraph: {
    icon: PenLine,
    summary: (ui, t) => {
      const pid = str(payload(ui), "pid")
      return pid === undefined
        ? t("agent:tools.rewrite_paragraph.summary")
        : t("agent:tools.rewrite_paragraph.summary_pid", { pid })
    },
  },
  delete_paragraphs: {
    icon: PenLine,
    summary: (ui, t) => {
      const deleted = strArray(payload(ui), "deleted")
      return t("agent:tools.delete_paragraphs.summary", { pids: deleted.join(", ") })
    },
  },
  finish_draft: {
    icon: ClipboardCheck,
    summary: (ui, t) => {
      const data = payload(ui)
      const confirmNeeded = data["confirm_needed"] === true
      const words = num(data, "word_count") ?? 0
      return confirmNeeded
        ? t("agent:tools.finish_draft.summary_needs_confirmation", { words })
        : t("agent:tools.finish_draft.summary", { words })
    },
  },
  finish_revision: {
    icon: ClipboardCheck,
    summary: (ui, t) => t("agent:tools.finish_revision.summary", { words: num(payload(ui), "word_count") ?? 0 }),
  },

  // Reviewer
  submit_review: {
    icon: ClipboardCheck,
    summary: (ui, t) => {
      const data = payload(ui)
      const verdict = str(data, "verdict")
      const issues = num(data, "issues") ?? 0
      return verdict === "pass"
        ? t("agent:tools.submit_review.summary_pass", { issues })
        : t("agent:tools.submit_review.summary_revise", { issues })
    },
  },

  // Archivist
  create_entity: { icon: Archive, summary: (ui, t) => archivistSummary("create_entity", ui, t) },
  update_entity: { icon: Archive, summary: (ui, t) => archivistSummary("update_entity", ui, t) },
  set_relation: { icon: Users, summary: (ui, t) => archivistSummary("set_relation", ui, t) },
  end_relation: { icon: Users, summary: (ui, t) => archivistSummary("end_relation", ui, t) },
  record_appearances: {
    icon: Users,
    summary: (ui, t) => archivistSummary("record_appearances", ui, t),
  },
  add_timeline_event: {
    icon: Compass,
    summary: (ui, t) => archivistSummary("add_timeline_event", ui, t),
  },
  update_timeline_event: {
    icon: Compass,
    summary: (ui, t) => archivistSummary("update_timeline_event", ui, t),
  },
  add_memory_note: { icon: FileText, summary: (ui, t) => archivistSummary("add_memory_note", ui, t) },
  retire_memory_note: {
    icon: FileText,
    summary: (ui, t) => archivistSummary("retire_memory_note", ui, t),
  },
  add_foreshadowing: {
    icon: Compass,
    summary: (ui, t) => archivistSummary("add_foreshadowing", ui, t),
  },
  resolve_foreshadowing: {
    icon: Compass,
    summary: (ui, t) => archivistSummary("resolve_foreshadowing", ui, t),
  },
  set_chapter_summary: {
    icon: FileText,
    summary: (ui, t) => archivistSummary("set_chapter_summary", ui, t),
  },
  propose_setting_change: {
    icon: FileText,
    summary: (ui, t) => {
      const target = str(payload(ui), "target")
      return target === undefined
        ? t("agent:tools.propose_setting_change.summary")
        : t("agent:tools.propose_setting_change.summary_target", { target })
    },
  },
  propose_goal_change: {
    icon: Target,
    summary: (ui, t) => {
      const data = payload(ui)
      const action = str(data, "action")
      const nodeId = num(data, "node_id")
      if (action === "complete") {
        return nodeId === undefined
          ? t("agent:tools.propose_goal_change.summary_complete")
          : t("agent:tools.propose_goal_change.summary_complete_node", { id: nodeId })
      }
      if (action === "drop") {
        return nodeId === undefined
          ? t("agent:tools.propose_goal_change.summary_drop")
          : t("agent:tools.propose_goal_change.summary_drop_node", { id: nodeId })
      }
      return t("agent:tools.propose_goal_change.summary_create")
    },
  },
  finish_maintenance: {
    icon: ClipboardCheck,
    summary: (ui, t) => {
      const data = payload(ui)
      return t("agent:tools.finish_maintenance.summary", {
        changes: num(data, "changes") ?? 0,
        proposals: num(data, "proposals") ?? 0,
      })
    },
  },

  // Conception
  ask_user: {
    icon: Sparkles,
    summary: (ui, t) => {
      const questions = payload(ui)["questions"]
      const count = Array.isArray(questions) ? questions.length : 0
      return t("agent:tools.ask_user.summary", { count })
    },
  },
  request_review: {
    icon: Sparkles,
    summary: (_ui, t) => t("agent:tools.request_review.summary"),
  },
  web_search: {
    icon: Search,
    summary: (ui, t) => {
      const data = payload(ui)
      const results = data["results"]
      const count = Array.isArray(results) ? results.length : 0
      const query = str(data, "query")
      return query === undefined
        ? t("agent:tools.web_search.summary", { count })
        : t("agent:tools.web_search.summary_query", { query, count })
    },
  },
  web_fetch: {
    icon: Download,
    summary: (ui, t) => {
      const data = payload(ui)
      const title = str(data, "title")
      const url = str(data, "url") ?? ""
      return title === undefined || title === ""
        ? t("agent:tools.web_fetch.summary", { url })
        : t("agent:tools.web_fetch.summary_title", { title })
    },
  },
  write_setting_doc: {
    icon: FileText,
    summary: (ui, t) => {
      const data = payload(ui)
      const doc = str(data, "doc")
      const chars = num(data, "chars") ?? 0
      return doc === undefined
        ? t("agent:tools.write_setting_doc.summary", { chars })
        : t("agent:tools.write_setting_doc.summary_doc", { doc: t(`agent:docs.${doc}`), chars })
    },
  },
  edit_setting_doc: {
    icon: FileText,
    summary: (ui, t) => {
      const doc = str(payload(ui), "doc")
      return doc === undefined
        ? t("agent:tools.edit_setting_doc.summary")
        : t("agent:tools.edit_setting_doc.summary_doc", { doc: t(`agent:docs.${doc}`) })
    },
  },
  upsert_character: {
    icon: Users,
    summary: (ui, t) => {
      const data = payload(ui)
      const name = str(data, "name") ?? ""
      const rank = num(data, "rank") ?? 0
      return t("agent:tools.upsert_character.summary", { name, rank })
    },
  },
  remove_character: {
    icon: Users,
    summary: (ui, t) => {
      const name = str(payload(ui), "name")
      return name === undefined
        ? t("agent:tools.remove_character.summary")
        : t("agent:tools.remove_character.summary_name", { name })
    },
  },
  reorder_characters: {
    icon: Users,
    summary: (ui, t) => {
      const order = strArray(payload(ui), "order")
      return t("agent:tools.reorder_characters.summary", { order: order.join(" > ") })
    },
  },
  set_research_topics: {
    icon: Search,
    summary: (ui, t) =>
      t("agent:tools.set_research_topics.summary", { count: num(payload(ui), "count") ?? 0 }),
  },

  // Researcher
  save_knowledge_entry: {
    icon: BookOpen,
    summary: (ui, t) => {
      const title = str(payload(ui), "title")
      return title === undefined
        ? t("agent:tools.save_knowledge_entry.summary")
        : t("agent:tools.save_knowledge_entry.summary_title", { title })
    },
  },
  update_knowledge_entry: {
    icon: BookOpen,
    summary: (ui, t) => {
      const title = str(payload(ui), "title")
      return title === undefined
        ? t("agent:tools.update_knowledge_entry.summary")
        : t("agent:tools.update_knowledge_entry.summary_title", { title })
    },
  },
  finish_research: {
    icon: ClipboardCheck,
    summary: (ui, t) => t("agent:tools.finish_research.summary", { entries: num(payload(ui), "entries") ?? 0 }),
  },

  // Book agent (D-169)
  book_overview: { icon: BookOpen, summary: (ui, t) => t("agent:tools.book_overview.summary", { count: num(payload(ui), "count") ?? 0 }) },
  open_chapter: { icon: BookOpen, summary: (ui, t) => {
    const chapter = num(payload(ui), "chapter_number")
    return chapter === undefined ? t("agent:tools.open_chapter.summary") : t("agent:tools.open_chapter.summary_chapter", { chapter })
  } },
  read_setting_doc: { icon: FileText, summary: (ui, t) => {
    const doc = str(payload(ui), "doc")
    return doc === undefined ? t("agent:tools.read_setting_doc.summary") : t("agent:tools.read_setting_doc.summary_doc", { doc: t(`agent:docs.${doc}`) })
  } },
  read_characters: { icon: Users, summary: (ui, t) => {
    const name = str(payload(ui), "name")
    return name === undefined ? t("agent:tools.read_characters.summary", { count: num(payload(ui), "count") ?? 0 }) : t("agent:tools.read_characters.summary_name", { name })
  } },
  list_goals: { icon: Target, summary: (ui, t) => t("agent:tools.list_goals.summary", { count: num(payload(ui), "count") ?? 0 }) },
  list_research: { icon: Search, summary: (ui, t) => t("agent:tools.list_research.summary", { count: num(payload(ui), "count") ?? 0 }) },
  list_sessions: { icon: List, summary: (ui, t) => t("agent:tools.list_sessions.summary", { count: num(payload(ui), "count") ?? 0 }) },
  create_chapter: { icon: FileText, summary: (_ui, t) => t("agent:tools.create_chapter.summary") },
  edit_chapter: { icon: FileText, summary: (_ui, t) => t("agent:tools.edit_chapter.summary") },
  edit_chapter_text: { icon: PenLine, summary: (_ui, t) => t("agent:tools.edit_chapter_text.summary") },
  delete_canon_record: { icon: Archive, summary: (ui, t) => t("agent:tools.delete_canon_record.summary", { type: str(payload(ui), "target_type") ?? "" }) },
  create_goal: { icon: Target, summary: (_ui, t) => t("agent:tools.create_goal.summary") },
  update_goal: { icon: Target, summary: (_ui, t) => t("agent:tools.update_goal.summary") },
  create_research_task: { icon: Search, summary: (_ui, t) => t("agent:tools.create_research_task.summary") },
  update_research_task: { icon: Search, summary: (_ui, t) => t("agent:tools.update_research_task.summary") },
  delete_research_task: { icon: Search, summary: (_ui, t) => t("agent:tools.delete_research_task.summary") },
  add_knowledge_entry: { icon: BookOpen, summary: (_ui, t) => t("agent:tools.add_knowledge_entry.summary") },
  start_session: { icon: Bot, summary: (ui, t) => {
    const sessionId = num(payload(ui), "session_id")
    return sessionId === undefined ? t("agent:tools.start_session.summary") : t("agent:tools.start_session.summary_started", { id: sessionId })
  } },

  // Test-only tools of the debug session (spec/10 §2.7).
  echo: {
    icon: Wrench,
    summary: (ui, t) => t("agent:tools.echo.summary", { chars: num(payload(ui), "chars") ?? 0 }),
  },
  test_write: {
    icon: Wrench,
    summary: (ui, t) => t("agent:tools.test_write.summary", { chars: num(payload(ui), "chars") ?? 0 }),
  },
  test_finish: {
    icon: ClipboardCheck,
    summary: (_ui, t) => t("agent:tools.test_finish.summary"),
  },
  test_slow: {
    icon: Download,
    summary: (ui, t) => t("agent:tools.test_slow.summary", { chars: num(payload(ui), "chars") ?? 0 }),
  },
}

/**
 * The archivist tools share one summary shape: the action's verb plus the target's name.
 *
 * Both keys need their `agent:` namespace: `t` here is the component's `t()`, whose default
 * namespace is `common`, so without the prefix i18next resolves nothing and the card shows the
 * literal key. Not every tool of the shape has a `summary_target` sibling
 * (`record_appearances` reports a count, not a target), so the target variant is only used where
 * the locale table declares one — `t` reports a missing key by returning it unchanged.
 */
function archivistSummary(name: string, ui: unknown, t: TFunction): string {
  const data = payload(ui)
  const target = str(data, "name") ?? str(data, "target") ?? ""
  const count = num(data, "count") ?? 0
  if (target !== "") {
    const variant = t(`agent:${summaryKey(name)}_target`, { target, count })
    // A tool without a target variant renders the bare summary instead of the unresolved key.
    if (!variant.startsWith("agent:tools.") && !variant.startsWith("tools.")) {
      return variant
    }
  }
  return t(summaryI18nKey(name), { count })
}

/** The renderer of a tool, or the generic fallback. */
export function toolRenderer(name: string): ToolRenderer {
  return REGISTRY[name] ?? DEFAULT
}

/** Every tool that has a dedicated renderer (used by the i18n parity test). */
export function registeredTools(): string[] {
  return Object.keys(REGISTRY)
}
