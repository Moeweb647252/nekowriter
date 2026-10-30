/** Pure helpers of the proposals UI (spec/06 §5, spec/10 §2.3). */
import type { ProposalDto } from "@/api/gen/ProposalDto"
import type { ProposalStatus } from "@/api/gen/ProposalStatus"

/** A proposal can be accepted only while it is `pending` (spec/10 §2.3). */
export function canAccept(status: ProposalStatus): boolean {
  return status === "pending"
}

/** Reject dismisses a `pending` or `conflict` proposal (spec/10 §2.3). */
export function canReject(status: ProposalStatus): boolean {
  return status === "pending" || status === "conflict"
}

/** The stored character name of a `new_character` proposal, when it has one. */
export function newCharacterName(proposal: ProposalDto): string | undefined {
  return proposal.edits.kind === "create_character" ? proposal.edits.character.name : undefined
}

/**
 * The i18n key of the proposal's target: the setting document's label, the linked character
 * design, or the new character's name (data, not a translated fragment).
 */
export function targetLabel(proposal: ProposalDto): { key: string; name?: string } {
  if (proposal.target_type === "setting_doc") {
    return { key: `book:setting_doc.${proposal.target_kind ?? "outline"}` }
  }
  if (proposal.target_type === "character_design") {
    return { key: "book:proposal.character_design" }
  }
  if (proposal.target_type === "goal_node") {
    return { key: "book:proposal.goal_node", name: proposal.goal_target?.title }
  }
  if (proposal.target_type === "new_goal_task") {
    return {
      key: "book:proposal.new_goal_task",
      name:
        proposal.edits.kind === "create_goal_task"
          ? proposal.edits.task.title
          : proposal.goal_target?.title,
    }
  }
  return { key: "book:proposal.new_character", name: newCharacterName(proposal) }
}
