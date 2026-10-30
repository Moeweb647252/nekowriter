import { describe, expect, it } from "vitest"

import type { ProposalDto } from "@/api/gen/ProposalDto"
import {
  canAccept,
  canReject,
  newCharacterName,
  targetLabel,
} from "@/features/proposals/proposalUtils"

function proposal(overrides: Partial<ProposalDto> = {}): ProposalDto {
  return {
    id: 1,
    book_id: 2,
    target_type: "setting_doc",
    target_kind: "world",
    target_id: null,
    edits: { kind: "append", text: "appended" },
    rationale: "because",
    status: "pending",
    chapter_id: 3,
    session_id: 4,
    created_at: "2026-01-01T00:00:00Z",
    decided_at: null,
    preview: { before: "before", after: "after" },
    goal_target: null,
    ...overrides,
  }
}

describe("proposalUtils", () => {
  it("accepts only pending proposals and rejects pending or conflicting ones", () => {
    expect(canAccept("pending")).toBe(true)
    expect(canAccept("conflict")).toBe(false)
    expect(canAccept("accepted")).toBe(false)
    expect(canReject("pending")).toBe(true)
    expect(canReject("conflict")).toBe(true)
    expect(canReject("rejected")).toBe(false)
    expect(canReject("accepted")).toBe(false)
  })

  it("names a new character from the stored edits", () => {
    const created = proposal({
      target_type: "new_character",
      target_kind: null,
      edits: {
        kind: "create_character",
        character: { name: "黑衣人", role_label: "", summary: "", profile: "" },
      },
    })
    expect(newCharacterName(created)).toBe("黑衣人")
    expect(newCharacterName(proposal())).toBeUndefined()
  })

  it("maps each target type to its localized label key", () => {
    expect(targetLabel(proposal())).toEqual({ key: "book:setting_doc.world" })
    expect(
      targetLabel(proposal({ target_type: "character_design", target_kind: null, target_id: 9 })),
    ).toEqual({ key: "book:proposal.character_design" })
    expect(
      targetLabel(
        proposal({
          target_type: "new_character",
          target_kind: null,
          edits: {
            kind: "create_character",
            character: { name: "柳照", role_label: "", summary: "", profile: "" },
          },
        }),
      ),
    ).toEqual({ key: "book:proposal.new_character", name: "柳照" })
    expect(
      targetLabel(
        proposal({
          target_type: "goal_node",
          target_kind: null,
          goal_target: { id: 3, level: "plan", title: "Take the sword", status: "active" },
        }),
      ),
    ).toEqual({ key: "book:proposal.goal_node", name: "Take the sword" })
    expect(
      targetLabel(
        proposal({
          target_type: "new_goal_task",
          target_kind: null,
          edits: {
            kind: "create_goal_task",
            task: {
              parent_id: null,
              title: "A new task",
              purpose: "why",
              chapter_from: null,
              chapter_to: null,
              entity_ids: [],
              event_ids: [],
              foreshadowing_ids: [],
              track_id: null,
            },
          },
        }),
      ),
    ).toEqual({ key: "book:proposal.new_goal_task", name: "A new task" })
  })
})
