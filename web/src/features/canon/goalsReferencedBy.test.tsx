/** Referenced-by is hidden while Goals is off and links to the node otherwise. */
import { render, screen } from "@testing-library/react"
import i18n from "i18next"
import { beforeAll, describe, expect, it, vi } from "vitest"

import enCanon from "@/i18n/locales/en/canon.json"

const state = vi.hoisted(() => ({ goals: true }))

vi.mock("react-router", () => ({
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => <a href={to}>{children}</a>,
}))

vi.mock("@/api/hooks/books", () => ({
  useBook: () => ({
    data: { settings: { canon_systems: { goals: state.goals } } },
    isPending: false,
    isError: false,
  }),
}))

vi.mock("@/api/hooks/goals", () => ({
  useGoals: () => ({
    data: [{ id: 4, title: "Find the sword" }],
    isPending: false,
    isError: false,
  }),
}))

import { GoalsReferencedBy } from "@/features/canon/GoalsReferencedBy"

beforeAll(() => {
  i18n.addResourceBundle("en", "canon", enCanon, true, true)
})

describe("GoalsReferencedBy", () => {
  it("links a referenced node while Goals is on", () => {
    state.goals = true
    render(<GoalsReferencedBy bookId={1} entityId={7} />)
    expect(screen.getByRole("heading", { name: "Referenced by" })).toBeInTheDocument()
    expect(screen.getByRole("link", { name: "Find the sword" })).toHaveAttribute(
      "href",
      "/books/1/canon/goals?node=4",
    )
  })

  it("renders nothing while Goals is off", () => {
    state.goals = false
    render(<GoalsReferencedBy bookId={1} entityId={7} />)
    expect(screen.queryByText("Referenced by")).not.toBeInTheDocument()
  })
})
