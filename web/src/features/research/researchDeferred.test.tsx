/**
 * The deferred research status in the UI (spec/10 §2.5, spec/11 §7, D-138).
 *
 * The author's problem was a dead end: the overview keeps `run_research` as the next action while
 * any task is `ready` (D-136), so without a "not now" the only ways out were to spend money on a
 * run or to throw the intent away. These tests pin the two buttons, the badge, and — the part that
 * actually unblocks the author — that the overview stops asking for research.
 */
import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import i18n from "i18next"
import { beforeAll, describe, expect, it, vi } from "vitest"

import enChapter from "@/i18n/locales/en/chapter.json"
import enKnowledge from "@/i18n/locales/en/knowledge.json"

import type { ResearchTaskDto } from "@/api/gen/ResearchTaskDto"

import { ResearchTaskList } from "./ResearchTaskList"
import ResearchTaskPage from "./ResearchTaskPage"

beforeAll(() => {
  for (const [namespace, bundle] of [
    ["knowledge", enKnowledge],
    ["chapter", enChapter],
  ] as const) {
    if (!i18n.hasResourceBundle("en", namespace)) {
      i18n.addResourceBundle("en", namespace, bundle)
    }
    const namespaces = (i18n.options.ns ?? []) as string[]
    if (!namespaces.includes(namespace)) {
      i18n.options.ns = [...namespaces, namespace]
    }
  }
})

function task(overrides: Partial<ResearchTaskDto> = {}): ResearchTaskDto {
  return {
    id: 1,
    book_id: 1,
    scope_id: 1,
    scope_name: "明初历史",
    topic: "宝钞与财政",
    kind: "topic",
    origin: "author",
    requirements: "",
    language: "zh-CN",
    status: "ready",
    session_id: null,
    report: "",
    entries_created: 0,
    entries: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  }
}

/** The hook state and mutation mocks; each test sets the row it renders and inspects its call. */
const state = vi.hoisted(() => ({
  task: null as ResearchTaskDto | null,
  tasks: [] as ResearchTaskDto[],
  deferMutate: vi.fn(),
  resumeMutate: vi.fn(),
  runMutate: vi.fn(),
  patchMutate: vi.fn(),
  deleteMutate: vi.fn(),
}))

vi.mock("@/api/hooks/research", () => {
  const idle = { isPending: false, mutate: vi.fn() }
  return {
    useResearchTasks: () => ({ isPending: false, data: state.tasks }),
    useResearchTask: () => ({ isPending: false, data: state.task }),
    useCreateResearchTask: () => idle,
    usePatchResearchTask: () => ({ isPending: false, mutate: state.patchMutate }),
    useDeleteResearchTask: () => ({ isPending: false, mutate: state.deleteMutate }),
    useRunResearchTask: () => ({ isPending: false, mutate: state.runMutate }),
    useDeferResearchTask: () => ({ isPending: false, mutate: state.deferMutate }),
    useResumeResearchTask: () => ({ isPending: false, mutate: state.resumeMutate }),
  }
})

vi.mock("@/api/hooks/sessions", () => ({
  useSession: () => ({ data: undefined }),
  useSessionControl: () => ({ isPending: false, mutate: vi.fn() }),
  useResumeSession: () => ({ isPending: false, mutate: vi.fn() }),
}))

vi.mock("react-router", () => ({
  useParams: () => ({ taskId: "1" }),
  useNavigate: () => vi.fn(),
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => (
    <a href={to}>{children}</a>
  ),
}))


describe("the research task page", () => {
  it("defers a ready task from the page", async () => {
    state.task = task()
    render(<ResearchTaskPage />)

    const button = screen.getByRole("button", { name: /defer/i })
    await userEvent.click(button)
    // The page asks before it changes the state of the author's task list.
    expect(state.deferMutate).toHaveBeenCalledWith(
      1,
      expect.objectContaining({ onError: expect.any(Function) }),
    )
  })

  it("offers resume instead of defer once the task is deferred", async () => {
    state.task = task({ status: "deferred" })
    render(<ResearchTaskPage />)

    expect(screen.queryByRole("button", { name: /^defer$/i })).toBeNull()
    const button = screen.getByRole("button", { name: /resume/i })
    await userEvent.click(button)
    expect(state.resumeMutate).toHaveBeenCalledWith(
      1,
      expect.objectContaining({ onError: expect.any(Function) }),
    )
  })

  it("shows the deferred badge with its own label", () => {
    state.task = task({ status: "deferred" })
    render(<ResearchTaskPage />)

    // The label comes from `chapter:status.deferred`, never from the wire value (spec/11 §7).
    expect(screen.getByText("Deferred")).toBeDefined()
  })

  it("hides both actions while a session is open", () => {
    // A task being researched cannot be deferred; the server refuses it and the UI does not offer
    // it (spec/07 §8 step 1).
    state.task = task({ status: "running", session_id: 9 })
    render(<ResearchTaskPage />)

    expect(screen.queryByRole("button", { name: /^defer$/i })).toBeNull()
    expect(screen.queryByRole("button", { name: /^resume$/i })).toBeNull()
  })

  it("does not offer defer on a draft or a finished task", () => {
    for (const status of ["draft", "done", "cancelled"] as const) {
      state.task = task({ status })
      const view = render(<ResearchTaskPage />)
      expect(
        screen.queryByRole("button", { name: /^defer$/i }),
        `defer is not offered on a ${status} task`,
      ).toBeNull()
      view.unmount()
    }
  })
})

describe("the research list", () => {
  it("shows the deferred badge for a deferred task", async () => {
    state.tasks = [task({ status: "deferred", topic: "宝钞与财政" })]
    render(<ResearchTaskList />)

    await waitFor(() => {
      expect(screen.getByText("Deferred")).toBeDefined()
    })
    expect(screen.getByText("宝钞与财政")).toBeDefined()
  })
})
