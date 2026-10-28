/** Writing-skill catalogue page (design §8): built-ins are preview-only, user rows are editable,
 * deleting names the books that used the skill. */
import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { WritingSkillDto } from "@/api/gen/WritingSkillDto"

const createSkill = vi.fn()
const patchSkill = vi.fn()
const deleteSkill = vi.fn()

let skills: WritingSkillDto[] = []

vi.mock("@/api/hooks/skills", () => ({
  useWritingSkills: () => ({ isPending: false, isError: false, data: skills }),
  useCreateWritingSkill: () => ({ mutate: createSkill, isPending: false }),
  usePatchWritingSkill: () => ({ mutate: patchSkill, isPending: false }),
  useDeleteWritingSkill: () => ({ mutate: deleteSkill, isPending: false }),
}))

vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))

import SkillsPage from "./SkillsPage"

const BUILTIN: WritingSkillDto = {
  id: 1,
  key: "basic-writing",
  name: "基础写作 / Basic Writing",
  category: "写作 / Writing",
  author: "builtin",
  description: "",
  body: "",
  enabled_by_default: false,
  retired: false,
  selected: false,
  used_by_books: [],
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
}

const USER: WritingSkillDto = {
  ...BUILTIN,
  id: 2,
  key: null,
  name: "冷硬文风",
  category: "文风",
  author: "user",
  description: "短句，少形容词。",
  body: "用短句推进。",
  used_by_books: [
    { id: 3, title: "青云劫" },
    { id: 4, title: "剑冢旧事" },
  ],
}

beforeAll(() => {
  Element.prototype.scrollIntoView = vi.fn()
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.setPointerCapture = vi.fn()
  Element.prototype.releasePointerCapture = vi.fn()
})

beforeEach(() => {
  createSkill.mockClear()
  patchSkill.mockClear()
  deleteSkill.mockClear()
  skills = [BUILTIN, USER]
})

describe("writing-skill catalogue", () => {
  it("shows the built-in badge and no edit or delete affordance on a built-in row", () => {
    render(<SkillsPage />)

    const builtin = screen.getByTestId("skill-1")
    expect(within(builtin).getByText("Built-in")).toBeInTheDocument()
    expect(within(builtin).getByRole("button", { name: "View" })).toBeInTheDocument()
    expect(within(builtin).queryByRole("button", { name: "Edit skill" })).toBeNull()
    expect(within(builtin).queryByRole("button", { name: "Delete skill" })).toBeNull()
  })

  it("previews a built-in body read-only", async () => {
    const user = userEvent.setup()
    render(<SkillsPage />)

    await user.click(within(screen.getByTestId("skill-1")).getByRole("button", { name: "View" }))
    const dialog = await screen.findByRole("dialog")
    expect(within(dialog).getByText("This skill has no body yet.")).toBeInTheDocument()
    expect(within(dialog).queryByRole("button", { name: "Save" })).toBeNull()
  })

  it("creates a user skill with the typed fields", async () => {
    const user = userEvent.setup()
    render(<SkillsPage />)

    await user.click(screen.getByRole("button", { name: "New skill" }))
    const dialog = await screen.findByRole("dialog")
    await user.type(within(dialog).getByLabelText("Name"), "  新技能 ")
    await user.type(within(dialog).getByLabelText("Category"), "文风")
    await user.type(within(dialog).getByLabelText("Instruction text"), "写短句。")
    await user.click(within(dialog).getByRole("button", { name: "Save" }))

    expect(createSkill).toHaveBeenCalledTimes(1)
    expect(createSkill.mock.calls[0][0]).toEqual({
      name: "新技能",
      category: "文风",
      description: "",
      body: "写短句。",
    })
  })

  it("edits a user skill", async () => {
    const user = userEvent.setup()
    render(<SkillsPage />)

    await user.click(within(screen.getByTestId("skill-2")).getByRole("button", { name: "Edit skill" }))
    const dialog = await screen.findByRole("dialog")
    const body = within(dialog).getByLabelText("Instruction text")
    await user.clear(body)
    await user.type(body, "改成更冷的短句。")
    await user.click(within(dialog).getByRole("button", { name: "Save" }))

    expect(patchSkill).toHaveBeenCalledTimes(1)
    expect(patchSkill.mock.calls[0][0]).toMatchObject({
      skillId: 2,
      patch: { name: "冷硬文风", body: "改成更冷的短句。" },
    })
  })

  it("names the books that use a user skill before deleting it", async () => {
    const user = userEvent.setup()
    render(<SkillsPage />)

    await user.click(
      within(screen.getByTestId("skill-2")).getByRole("button", { name: "Delete skill" }),
    )
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("These books use it:")
    expect(dialog).toHaveTextContent("青云劫")
    expect(dialog).toHaveTextContent("剑冢旧事")
    await user.click(within(dialog).getByRole("button", { name: "Delete" }))
    expect(deleteSkill).toHaveBeenCalledWith(2, expect.any(Object))
  })

  it("shows the catalogue empty state, not a skill body's empty state", () => {
    skills = []
    render(<SkillsPage />)

    expect(screen.getByText("No writing skills yet")).toBeInTheDocument()
    expect(
      screen.getByText(
        "Built-in skills ship empty. Write your own to tell the writer how this book's prose should read.",
      ),
    ).toBeInTheDocument()
    expect(screen.queryByText("This skill has no body yet.")).toBeNull()
  })
})
