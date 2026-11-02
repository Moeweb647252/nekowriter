/** Session endpoints (spec/10 §2.7). */
import { api } from "@/api/client"
import type { DebugEchoDto } from "@/api/gen/DebugEchoDto"
import type { MessagePageDto } from "@/api/gen/MessagePageDto"
import type { ResumeInput } from "@/api/gen/ResumeInput"
import type { SessionAnswerInput } from "@/api/gen/SessionAnswerInput"
import type { SessionDto } from "@/api/gen/SessionDto"
import type { SessionMessageInput } from "@/api/gen/SessionMessageInput"
import type { SessionPatch } from "@/api/gen/SessionPatch"
import type { SessionSnapshotDto } from "@/api/gen/SessionSnapshotDto"

export function listSessions(bookId: number, filters?: string): Promise<SessionDto[]> {
  const query = filters === undefined ? "" : `?${filters}`
  return api.get<SessionDto[]>(`/books/${bookId}/sessions${query}`)
}

export function getSession(sessionId: number): Promise<SessionSnapshotDto> {
  return api.get<SessionSnapshotDto>(`/sessions/${sessionId}`)
}

export function getMessages(sessionId: number, cursor?: string, limit?: number): Promise<MessagePageDto> {
  const params = new URLSearchParams()
  if (cursor !== undefined) {
    params.set("cursor", cursor)
  }
  if (limit !== undefined) {
    params.set("limit", String(limit))
  }
  const query = params.size === 0 ? "" : `?${params.toString()}`
  return api.get<MessagePageDto>(`/sessions/${sessionId}/messages${query}`)
}

/** One control operation (`pause`, `resume`, `stop`, `continue`, `abandon`). */
export function control(
  sessionId: number,
  operation: "pause" | "resume" | "stop" | "continue" | "abandon",
  body: unknown = {},
): Promise<SessionDto> {
  return api.post<SessionDto>(`/sessions/${sessionId}/${operation}`, body)
}

export function resumeSession(sessionId: number, input: ResumeInput): Promise<SessionDto> {
  return api.post<SessionDto>(`/sessions/${sessionId}/resume`, input)
}

export function sendSessionMessage(
  sessionId: number,
  input: SessionMessageInput,
): Promise<SessionDto> {
  return api.post<SessionDto>(`/sessions/${sessionId}/message`, input)
}

export function answerSession(
  sessionId: number,
  input: SessionAnswerInput,
): Promise<SessionDto> {
  return api.post<SessionDto>(`/sessions/${sessionId}/answer`, input)
}

export function patchSession(sessionId: number, patch: SessionPatch): Promise<SessionDto> {
  return api.patch<SessionDto>(`/sessions/${sessionId}`, patch)
}

/** Starts the debug session of spec/10 §2.7 (debug builds only). */
export function startDebugSession(bookId: number): Promise<DebugEchoDto> {
  return api.post<DebugEchoDto>("/debug/sessions/echo", { book_id: bookId })
}

/** True when this build exposes the debug session endpoint (spec/10 §2.7). */
export const debugSessionsEnabled = import.meta.env.DEV
