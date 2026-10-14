import type { PromptKey } from "@/api/generated/PromptKey"
import type { PromptRole } from "@/api/generated/PromptRole"
import type { PromptVariable } from "@/api/generated/PromptVariable"

export default {
  title: "Prompts",
  description:
    "The instructions each agent follows. A work's own prompt wins over the system's, which wins over the built-in text. Built-in prompts are written in Chinese; the agents write the novel in its writing language.",
  scope: "Applies to",
  allWorks: "All works",
  list: "Prompts",
  groups: {
    writing: "Writing",
    review: "Review",
  },
  keys: {
    "preparation.generate": "Preparation",
    "chapter.outline": "Chapter outline",
    "chapter.write": "Chapter text",
    "review.preparation.background": "Review: background",
    "review.preparation.research": "Review: research",
    "review.preparation.characters": "Review: characters",
    "review.preparation.outline": "Review: outline",
    "review.chapter.outline": "Review: chapter outline",
    "review.chapter.edit": "Review: single edit",
    "review.chapter.final": "Review: whole chapter",
  } satisfies Record<PromptKey, string>,
  roles: {
    system: "System prompt",
    user: "User prompt",
  } satisfies Record<PromptRole, string>,
  source: {
    builtin: "the built-in text",
    system: "the system's prompt",
    project: "this work's prompt",
  },
  inEffect: "In effect: {{source}}.",
  text: "Prompt text",
  textHint:
    "The agent adds the stage's material and its tool instructions after this text. Placeholders in double braces are filled in for every request.",
  save: "Save",
  saved: "Saved",
  reset: "Reset to {{source}}",
  conflict: "This prompt changed after this draft was started. Check it before saving.",
  preview: "Preview with examples",
  previewTitle: "Preview",
  variables: "Placeholders",
  required: "Required",
  optional: "Optional",
  example: "Example: {{example}}",
  variable: {
    writing_language: {
      help: "The language the novel is written in.",
      example: "English",
    },
    novel_brief: {
      help: "The novel brief the user wrote.",
      example: "A ghost story in a lighthouse on a cold coast.",
    },
    required_artifacts: {
      help: "The preparation documents to write in this round, in order.",
      example: "background, research, characters, outline",
    },
    chapter_kind: {
      help: "Whether the chapter belongs to the plan or is an extra chapter.",
      example: "chapter",
    },
    chapter_number: {
      help: "The chapter's number among the chapters of its kind.",
      example: "3",
    },
    chapter_title: {
      help: "The chapter's title.",
      example: "The Storm",
    },
    chapter_goal: {
      help: "The goal of the chapter in the chapter plan; empty for extra chapters.",
      example: "The lamp fails during the storm.",
    },
    chapter_guidance: {
      help: "What the user asked of the chapter before it was written; may be empty.",
      example: "Keep the keeper silent until the end.",
    },
  } satisfies Record<PromptVariable, { help: string; example: string }>,
}
