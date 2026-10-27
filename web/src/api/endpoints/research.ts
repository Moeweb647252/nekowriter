/** Research task endpoints (spec/10 §2.5). */
import { api } from "@/api/client"
import type { ResearchRunResponse } from "@/api/gen/ResearchRunResponse"
import type { ResearchTaskCreate } from "@/api/gen/ResearchTaskCreate"
import type { ResearchTaskDto } from "@/api/gen/ResearchTaskDto"
import type { ResearchTaskPatch } from "@/api/gen/ResearchTaskPatch"
import type { ResearchTaskStatus } from "@/api/gen/ResearchTaskStatus"

export function listResearchTasks(params: {
  bookId?: number
  status?: ResearchTaskStatus
} = {}): Promise<ResearchTaskDto[]> {
  const query = new URLSearchParams()
  if (params.bookId !== undefined) query.set("book_id", String(params.bookId))
  if (params.status !== undefined) query.set("status", params.status)
  const suffix = query.size === 0 ? "" : `?${query.toString()}`
  return api.get<ResearchTaskDto[]>(`/research-tasks${suffix}`)
}

export function getResearchTask(taskId: number): Promise<ResearchTaskDto> {
  return api.get<ResearchTaskDto>(`/research-tasks/${taskId}`)
}

export function createResearchTask(input: ResearchTaskCreate): Promise<ResearchTaskDto> {
  return api.post<ResearchTaskDto>("/research-tasks", input)
}

export function patchResearchTask(
  taskId: number,
  patch: ResearchTaskPatch,
): Promise<ResearchTaskDto> {
  return api.patch<ResearchTaskDto>(`/research-tasks/${taskId}`, patch)
}

export function deleteResearchTask(taskId: number): Promise<void> {
  return api.delete<void>(`/research-tasks/${taskId}`)
}

export function runResearchTask(taskId: number): Promise<ResearchRunResponse> {
  return api.post<ResearchRunResponse>(`/research-tasks/${taskId}/run`, {})
}

/**
 * Postpones a ready task without running or cancelling it (spec/10 §2.5, D-138). Idempotent: a
 * second call returns the current row instead of failing, so a double-click is harmless.
 */
export function deferResearchTask(taskId: number): Promise<ResearchTaskDto> {
  return api.post<ResearchTaskDto>(`/research-tasks/${taskId}/defer`, {})
}

/** Returns a deferred task to `ready`, making it runnable again (spec/10 §2.5, D-138). */
export function resumeResearchTask(taskId: number): Promise<ResearchTaskDto> {
  return api.post<ResearchTaskDto>(`/research-tasks/${taskId}/resume`, {})
}
