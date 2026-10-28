/** Reopens the agent panel when a session is focused (the panel is collapsible). */
import type { BookShell } from "@/features/workspace/shell"

export function focusSessionInAgentPanel(shell: BookShell | null): void {
  shell?.expandAgentPanel()
}
