/**
 * Pure-function tests for the conception studio (spec/05 §7 payload shapes, PROJECT §8.7).
 *
 * The forms' payloads are the protocol contract with the server's answer validation, so they are
 * tested without rendering: a wrong field name would surface as a 400 the author cannot fix.
 */
import { describe, expect, it } from "vitest"

import type { MessageWithCallsDto } from "@/api/gen/MessageWithCallsDto"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { buildAnswerPayload, readQuestions } from "@/features/conception/AskUserForm"
import { awaitingCall, materialsComplete } from "@/features/conception/ConceptionStudio"
import {
  approveAllItems,
  buildReviewPayload,
  initialReviewState,
  REVIEW_ITEMS,
  toggleVerdict,
  unreviewedItems,
  verdictOf,
  withVerdict,
} from "@/features/conception/ReviewRequestForm"

const questionsUi = {
  questions: [
    {
      id: "tone",
      question: "整体基调？",
      options: [{ label: "热血" }, { label: "沉郁" }],
      multi_select: false,
      allow_other: true,
    },
    {
      id: "tags",
      question: "想要哪些元素？",
      options: [{ label: "江湖" }, { label: "朝堂" }],
      multi_select: true,
      allow_other: false,
    },
  ],
}

describe("readQuestions", () => {
  it("reads the recorded questions and defaults", () => {
    const questions = readQuestions(questionsUi)
    expect(questions).toHaveLength(2)
    expect(questions[0].id).toBe("tone")
    expect(questions[0].options.map((option) => option.label)).toEqual(["热血", "沉郁"])
    expect(questions[0].allow_other).toBe(true)
    expect(questions[1].multi_select).toBe(true)
  })

  it("ignores a malformed payload instead of throwing", () => {
    expect(readQuestions(null)).toEqual([])
    expect(readQuestions({ questions: [{ question: "no id" }] })).toEqual([])
    expect(readQuestions({ questions: "nope" })).toEqual([])
  })
})

describe("buildAnswerPayload", () => {
  it("emits {answers: [{id, selected, other?}]} exactly (spec/05 §7)", () => {
    const questions = readQuestions(questionsUi)
    const payload = buildAnswerPayload(questions, {
      selected: { tone: ["沉郁"], tags: ["江湖", "朝堂"] },
      other: { tone: "  像旧信一样安静  ", tags: "" },
    })
    expect(payload).toEqual({
      answers: [
        { id: "tone", selected: ["沉郁"], other: "像旧信一样安静" },
        { id: "tags", selected: ["江湖", "朝堂"] },
      ],
    })
  })

  it("reports an unanswered question as an empty selection", () => {
    const questions = readQuestions(questionsUi)
    expect(buildAnswerPayload(questions, { selected: {}, other: {} })).toEqual({
      answers: [
        { id: "tone", selected: [] },
        { id: "tags", selected: [] },
      ],
    })
  })
})

describe("buildReviewPayload", () => {
  it("carries one verdict per reviewed item plus the general comment", () => {
    const payload = buildReviewPayload(
      {
        outline: { approved: true, feedback: "" },
        world: { approved: false, feedback: " 需要更具体 " },
        characters: { approved: true, feedback: "" },
      },
      "整体方向可以",
    )
    expect(payload).toEqual({
      docs: {
        outline: { approved: true },
        world: { approved: false, feedback: "需要更具体" },
        characters: { approved: true },
      },
      general: "整体方向可以",
    })
  })

  it("omits an empty general comment", () => {
    const payload = buildReviewPayload({ outline: { approved: true, feedback: "" } }, "   ")
    expect(payload).toEqual({ docs: { outline: { approved: true } } })
  })

  it("omits an item that was never given a verdict", () => {
    // `docs`' items are optional (spec/05 §7), so an unset item is left out rather than sent as a
    // default — the form itself refuses to submit while one is left (D-134).
    const payload = buildReviewPayload(
      { outline: { approved: true, feedback: "" }, world: { approved: null, feedback: "" } },
      "",
    )
    expect(payload).toEqual({ docs: { outline: { approved: true } } })
  })

  it("never carries feedback for an approved item", () => {
    // Approving hides the feedback box but keeps the draft, so a stray comment typed while the item
    // needed changes must not be sent as an approval condition (the model renders it either way).
    const payload = buildReviewPayload(
      { world: { approved: true, feedback: " 需要更具体 " } },
      "",
    )
    expect(payload).toEqual({ docs: { world: { approved: true } } })
  })

  it("carries the final verdict after approve → needs changes → approve", () => {
    // The author may change their mind on one row before submitting; the payload is built once, from
    // the live state, so it must carry the last click for each item.
    let state = initialReviewState(["outline", "world"])
    state = withVerdict(state, "outline", true)
    state = withVerdict(state, "outline", false)
    state = { ...state, outline: { ...verdictOf(state, "outline"), feedback: "冲突太薄" } }
    state = withVerdict(state, "outline", true)
    state = toggleVerdict(state, "world", false)

    expect(buildReviewPayload(state, "")).toEqual({
      docs: { outline: { approved: true }, world: { approved: false } },
    })
  })

  it("starts every item unset, so the form has not answered the review", () => {
    const state = initialReviewState(REVIEW_ITEMS)
    expect(Object.keys(state)).toEqual([...REVIEW_ITEMS])
    for (const item of REVIEW_ITEMS) {
      expect(verdictOf(state, item)).toEqual({ approved: null, feedback: "" })
    }
    expect(unreviewedItems(state, REVIEW_ITEMS)).toEqual([...REVIEW_ITEMS])
    // The regression guard in payload terms: a fresh form submits nothing at all.
    expect(buildReviewPayload(state, "")).toEqual({ docs: {} })
  })

  it("covers every reviewable item the studio exposes", () => {
    expect(REVIEW_ITEMS).toEqual([
      "outline",
      "characters",
      "world",
      "protagonist",
      "research_topics",
    ])
  })
})

describe("review verdict transitions", () => {
  it("keeps a rejection's feedback draft when the verdict is toggled and restored", () => {
    let state = withVerdict(initialReviewState(["outline"]), "outline", false)
    state = { ...state, outline: { ...verdictOf(state, "outline"), feedback: "thin" } }
    state = withVerdict(state, "outline", true)
    // The text survives the approval (the box is hidden, not destroyed)…
    expect(verdictOf(state, "outline")).toEqual({ approved: true, feedback: "thin" })
    // …and is back, unchanged, if the author returns to needs-changes.
    state = withVerdict(state, "outline", false)
    expect(verdictOf(state, "outline")).toEqual({ approved: false, feedback: "thin" })
  })

  it("releases an already-selected verdict back to unset instead of ignoring the click", () => {
    let state = withVerdict(initialReviewState(["outline"]), "outline", true)
    expect(verdictOf(state, "outline").approved).toBe(true)
    state = toggleVerdict(state, "outline", true)
    expect(verdictOf(state, "outline").approved).toBeNull()
    // The opposite verdict on a selected row is a normal verdict change, not a release.
    state = toggleVerdict(state, "outline", true)
    state = toggleVerdict(state, "outline", false)
    expect(verdictOf(state, "outline").approved).toBe(false)
  })

  it("approves every item without dropping a rejection's draft off the other items", () => {
    let state = withVerdict(initialReviewState(["outline", "world"]), "world", false)
    state = { ...state, world: { ...verdictOf(state, "world"), feedback: "more factions" } }
    state = approveAllItems(state, ["outline", "world"])
    expect(unreviewedItems(state, ["outline", "world"])).toEqual([])
    expect(verdictOf(state, "outline").approved).toBe(true)
    // Approve-all is a verdict change, not a state wipe: the draft is still there should the author
    // go back to needs-changes on that row.
    expect(verdictOf(state, "world")).toEqual({ approved: true, feedback: "more factions" })
  })
})

describe("materialsComplete", () => {
  const docs = [
    { kind: "outline", content: "# 大纲" },
    { kind: "world", content: "# 世界" },
    { kind: "protagonist", content: "# 主角" },
  ]

  it("requires the three documents and at least one character", () => {
    expect(materialsComplete({ docs, characters: [{ id: 1 }] })).toBe(true)
    expect(materialsComplete({ docs, characters: [] })).toBe(false)
    expect(
      materialsComplete({
        docs: [{ kind: "outline", content: "# 大纲" }, { kind: "world", content: "  " }, { kind: "protagonist", content: "# 主角" }],
        characters: [{ id: 1 }],
      }),
    ).toBe(false)
  })
})

/** A minimal session for the awaiting-call helper. */
function session(status: string, kind = "conception"): SessionDto {
  return { id: 7, status, kind } as SessionDto
}

function call(overrides: Partial<ToolCallDto>): ToolCallDto {
  return {
    id: 3,
    message_id: 1,
    seq: 0,
    call_index: 0,
    name: "ask_user",
    input: {},
    input_text: "{}",
    status: "awaiting_input",
    is_error: false,
    result_text: null,
    appendices: [],
    ui: questionsUi,
    sealed: false,
    created_at: "2026-01-01T00:00:00Z",
    finished_at: null,
    ...overrides,
  } as ToolCallDto
}

function messages(calls: ToolCallDto[]): MessageWithCallsDto[] {
  return [{ message: { id: 1 } as MessageWithCallsDto["message"], tool_calls: calls }]
}

describe("awaitingCall", () => {
  it("finds the call whose status waits for the author", () => {
    const found = awaitingCall(session("awaiting_input"), messages([call({})]))
    expect(found?.name).toBe("ask_user")
  })

  it("returns null when the session is not awaiting input", () => {
    expect(awaitingCall(session("running"), messages([call({})]))).toBeNull()
    expect(awaitingCall(session("idle"), messages([call({})]))).toBeNull()
  })

  it("ignores calls that are not awaiting input themselves", () => {
    expect(
      awaitingCall(session("awaiting_input"), messages([call({ status: "done" })])),
    ).toBeNull()
  })
})
