/**
 * The book workspace shell that routed pages render inside (PROJECT §8.4, spec/11 §5.1).
 *
 * `BookLayout` provides it through React Router's outlet context. It is its own module so a page can
 * read the shell without importing the layout (which pulls in the navigator and the agent panel).
 */
import { useOutletContext } from "react-router"

import type { BookDto } from "@/api/gen/BookDto"

export type BookShell = {
  book: BookDto
  /** Whether the agent panel is collapsed right now (it is `collapsible`, spec/11 §5.1). */
  agentPanelCollapsed: boolean
  /**
   * Opens the agent panel. A long agent run is watched there (PROJECT §8.4, spec/11 §5.3), so
   * starting one reopens the panel if the author had collapsed it. A no-op when it is open.
   */
  expandAgentPanel: () => void
}

/** The workspace shell, or `null` when the page is rendered outside a book workspace. */
export function useBookShell(): BookShell | null {
  return useOutletContext<BookShell | null>()
}
