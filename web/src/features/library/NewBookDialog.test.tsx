/** New book dialog: conception checkbox, knowledge-check auto-start and landing (D-158). */
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { MemoryRouter } from "react-router"
import { beforeEach, describe, expect, it, vi } from "vitest"

const create = vi.fn()
const navigate = vi.fn()

vi.mock("@/api/hooks/books", () => ({
  useCreateBook: () => ({ mutate: create, isPending: false }),
}))
vi.mock("@/components/common/ErrorToast", () => ({ useErrorToast: () => vi.fn() }))
vi.mock("react-router", () => ({
  MemoryRouter: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useNavigate: () => navigate,
}))

import { NewBookDialog } from "./NewBookDialog"

function renderDialog() {
  return render(
    <MemoryRouter>
      <NewBookDialog open onOpenChange={vi.fn()} />
    </MemoryRouter>,
  )
}

function lastInput() {
  expect(create).toHaveBeenCalledTimes(1)
  return create.mock.calls[0][0] as Record<string, unknown>
}

beforeEach(() => {
  create.mockReset()
  navigate.mockReset()
})

describe("NewBookDialog knowledge check", () => {
  it("hides the auto-start checkbox when conception is off and sends both fields when on", async () => {
    const user = userEvent.setup()
    const view = renderDialog()
    const auto = screen.getByLabelText("Start conception automatically after the knowledge check")
    expect(auto).toBeInTheDocument()
    await user.click(screen.getByLabelText("Start with the conception agent"))
    expect(
      screen.queryByLabelText("Start conception automatically after the knowledge check"),
    ).not.toBeInTheDocument()
    await user.click(screen.getByLabelText("Start with the conception agent"))
    await user.type(screen.getByLabelText("Title"), "A book")
    await user.click(screen.getByRole("button", { name: "Create book" }))
    expect(lastInput()).toMatchObject({ start_conception: true, knowledge_check: true, auto_start_conception: true })
    view.unmount()
  })

  it("sends auto_start_conception false when the author unchecks it", async () => {
    const user = userEvent.setup()
    renderDialog()
    await user.type(screen.getByLabelText("Title"), "A book")
    await user.click(screen.getByLabelText("Start conception automatically after the knowledge check"))
    await user.click(screen.getByRole("button", { name: "Create book" }))
    expect(lastInput()).toMatchObject({ knowledge_check: true, auto_start_conception: false })
  })

  it("lands on the studio for a conceiving book even without a session id", async () => {
    create.mockImplementation((_input: unknown, options: { onSuccess: (response: unknown) => void }) => {
      options.onSuccess({ book: { id: 7, status: "conceiving" }, session_id: null })
    })
    const user = userEvent.setup()
    renderDialog()
    await user.type(screen.getByLabelText("Title"), "A book")
    await user.click(screen.getByRole("button", { name: "Create book" }))
    expect(navigate).toHaveBeenCalledWith("/books/7/conception")
  })

  it("lands on the overview for a non-conceiving book", async () => {
    create.mockImplementation((_input: unknown, options: { onSuccess: (response: unknown) => void }) => {
      options.onSuccess({ book: { id: 8, status: "active" }, session_id: null })
    })
    const user = userEvent.setup()
    renderDialog()
    await user.type(screen.getByLabelText("Title"), "A book")
    await user.click(screen.getByRole("button", { name: "Create book" }))
    expect(navigate).toHaveBeenCalledWith("/books/8")
  })
})
