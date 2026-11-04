/** The session header's model/effort line (spec/11 §5.3, D-162). */
import { render, screen } from "@testing-library/react"
import i18n from "i18next"
import { beforeAll, describe, expect, it, vi } from "vitest"

import enAgent from "@/i18n/locales/en/agent.json"
import enSettings from "@/i18n/locales/en/settings.json"
import type { SessionDto } from "@/api/gen/SessionDto"
import { SessionHeader } from "@/features/agent/SessionHeader"

beforeAll(() => {
  if (!i18n.hasResourceBundle("en", "agent")) {
    i18n.addResourceBundle("en", "agent", enAgent)
  }
  if (!i18n.hasResourceBundle("en", "settings")) {
    i18n.addResourceBundle("en", "settings", enSettings)
  }
  if (!i18n.options.ns?.includes("settings")) {
    i18n.options.ns = [...(i18n.options.ns as string[]), "settings"]
  }
})

function session(overrides: Partial<SessionDto> = {}): SessionDto {
  return {
    id: 1,
    book_id: 1,
    role: "writer",
    kind: "chapter_write",
    chapter_id: 1,
    track_id: 1,
    research_task_id: null,
    chapter_number: 1,
    research_topic: null,
    model_label: "Demo model",
    context_window: 100000,
    effort: null,
    status: "running",
    phase: "drafting",
    granularity: "paragraph",
    auto_pause: false,
    pause_after_outline: true,
    pause_requested: false,
    held_tool_call_id: null,
    base_content_version: 0,
    error: null,
    usage: {
      requests: 0,
      input_tokens: 0,
      cached_input_tokens: 0,
      cache_write_tokens: 0,
      output_tokens: 0,
      reasoning_tokens: 0,
      last_prompt_tokens: 0,
      estimated: false,
    },
    counters: {
      steps: 0,
      tool_calls: 0,
      retrieval_calls: 0,
      consecutive_tool_errors: 0,
      consecutive_nudges: 0,
      finish_warned: false,
      research_budget_noticed: false,
    },
    ...overrides,
  } as SessionDto
}

describe("SessionHeader effort", () => {
  it("shows the localized effort after the model label", () => {
    render(
      <SessionHeader
        session={session({ effort: "high" })}
        sessions={[]}
        onSelect={vi.fn()}
        systemPrompt=""
        toolNames={[]}
      />,
    )
    expect(screen.getByText("Demo model · High")).toBeInTheDocument()
  })

  it("does not add a separator when the session has no effort", () => {
    render(
      <SessionHeader
        session={session()}
        sessions={[]}
        onSelect={vi.fn()}
        systemPrompt=""
        toolNames={[]}
      />,
    )
    expect(screen.getByText("Demo model")).toBeInTheDocument()
  })
})
