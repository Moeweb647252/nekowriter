/**
 * The `ask_user` form (PROJECT §8.7, spec/11 §5.3) — checked for the defect class of task-41:
 * a control that opens in its "selected" visual state, so the first click on it changes nothing
 * and reads as dead.
 *
 * The review form had that bug (every item started `approved: true`, so 「通过」 was a no-op on a
 * fresh row). This form does not: every radio and checkbox starts unchecked, so the first click is
 * always an observable change. These tests pin that down, so a future "sensible default" added here
 * cannot reintroduce the pattern silently.
 */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it, vi } from "vitest"

import { AskUserForm, readQuestions } from "@/features/conception/AskUserForm"

const QUESTIONS = readQuestions({
  questions: [
    {
      id: "tone",
      question: "Overall tone?",
      options: [{ label: "Heat" }, { label: "Gloom" }],
      multi_select: false,
      allow_other: true,
    },
    {
      id: "tags",
      question: "Which elements?",
      options: [{ label: "Jianghu" }, { label: "Court" }],
      multi_select: true,
      allow_other: false,
    },
  ],
})

function renderForm() {
  const onSubmit = vi.fn()
  render(<AskUserForm questions={QUESTIONS} pending={false} onSubmit={onSubmit} />)
  return onSubmit
}

const radios = () => screen.getAllByRole("radio")
const checkboxes = () => screen.getAllByRole("checkbox")

describe("AskUserForm — nothing starts pre-selected", () => {
  it("opens with every radio and checkbox unchecked", () => {
    renderForm()
    // No control is in its selected state at mount, which is what makes the first click visible.
    expect(radios()).toHaveLength(2)
    for (const radio of radios()) {
      expect(radio).not.toBeChecked()
    }
    for (const checkbox of checkboxes()) {
      expect(checkbox).not.toBeChecked()
    }
  })

  it("checks the radio the author clicks, and moves the selection off the previous one", async () => {
    renderForm()
    await userEvent.click(screen.getByRole("radio", { name: "Gloom" }))
    expect(screen.getByRole("radio", { name: "Gloom" })).toBeChecked()
    expect(screen.getByRole("radio", { name: "Heat" })).not.toBeChecked()

    // Clicking the other option is a real change, not a no-op.
    await userEvent.click(screen.getByRole("radio", { name: "Heat" }))
    expect(screen.getByRole("radio", { name: "Heat" })).toBeChecked()
    expect(screen.getByRole("radio", { name: "Gloom" })).not.toBeChecked()
  })

  it("toggles a multi-select checkbox both ways", async () => {
    renderForm()
    const jianghu = screen.getByRole("checkbox", { name: "Jianghu" })
    await userEvent.click(jianghu)
    expect(jianghu).toBeChecked()
    await userEvent.click(jianghu)
    expect(jianghu).not.toBeChecked()
  })

  it("sends the selection the author made", async () => {
    const onSubmit = renderForm()
    await userEvent.click(screen.getByRole("radio", { name: "Gloom" }))
    await userEvent.click(screen.getByRole("checkbox", { name: "Court" }))
    await userEvent.click(screen.getByRole("button", { name: "Submit answers" }))
    expect(onSubmit).toHaveBeenCalledWith({
      answers: [
        { id: "tone", selected: ["Gloom"] },
        { id: "tags", selected: ["Court"] },
      ],
    })
  })
})
