import type { RunStatus } from "@/api/generated/RunStatus"

/** Whether the run ended its execution with a problem the user decides about. */
export function isHalted(status: RunStatus): boolean {
  return status === "interrupted" || status === "stopped" || status === "failed"
}
