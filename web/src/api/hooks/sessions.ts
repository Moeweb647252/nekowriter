/** Session queries and mutations (spec/11 §4.1, §5.3). */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/sessions"
import { keys } from "@/api/keys"
import type { ResumeInput } from "@/api/gen/ResumeInput"
import type { SessionAnswerInput } from "@/api/gen/SessionAnswerInput"
import type { SessionMessageInput } from "@/api/gen/SessionMessageInput"
import type { SessionPatch } from "@/api/gen/SessionPatch"

/** The open session snapshot; the activity stream and controls read it. */
export function useSession(sessionId: number | null) {
  return useQuery({
    queryKey: keys.sessions.detail(sessionId ?? 0),
    queryFn: () => endpoints.getSession(sessionId ?? 0),
    enabled: sessionId !== null && sessionId > 0,
  })
}

/** The book's sessions, for the switcher (spec/11 §5.3). */
export function useBookSessions(bookId: number) {
  return useQuery({
    queryKey: keys.sessions.list(bookId),
    queryFn: () => endpoints.listSessions(bookId),
    enabled: bookId > 0,
  })
}

/** Scrollback: older messages by cursor (spec/11 §5.3). */
export function useOlderMessages(sessionId: number | null, cursor: string | undefined) {
  return useQuery({
    queryKey: [...keys.sessions.detail(sessionId ?? 0), "messages", cursor ?? "start"] as const,
    queryFn: () => endpoints.getMessages(sessionId ?? 0, cursor),
    enabled: sessionId !== null && sessionId > 0 && cursor !== undefined,
  })
}

/** One control operation; the response replaces the cached session. */
export function useSessionControl(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ sessionId, operation }: { sessionId: number; operation: "pause" | "resume" | "stop" | "continue" | "abandon" }) =>
      endpoints.control(sessionId, operation),
    onSuccess: (session) => {
      queryClient.setQueryData(keys.sessions.detail(session.id), (previous: unknown) => {
        if (previous !== undefined && previous !== null && typeof previous === "object") {
          return { ...previous, session }
        }
        return previous
      })
      void queryClient.invalidateQueries({ queryKey: keys.sessions.detail(session.id) })
      void queryClient.invalidateQueries({ queryKey: keys.sessions.list(bookId) })
    },
  })
}

export function useResumeSession(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ sessionId, feedback }: { sessionId: number; feedback?: string }) => {
      const input: ResumeInput = feedback === undefined || feedback === "" ? {} : { feedback }
      return endpoints.resumeSession(sessionId, input)
    },
    onSuccess: (session) => {
      void queryClient.invalidateQueries({ queryKey: keys.sessions.detail(session.id) })
      void queryClient.invalidateQueries({ queryKey: keys.sessions.list(bookId) })
    },
  })
}

export function useSendSessionMessage(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ sessionId, text }: { sessionId: number; text: string }) => {
      const input: SessionMessageInput = { text }
      return endpoints.sendSessionMessage(sessionId, input)
    },
    onSuccess: (session) => {
      void queryClient.invalidateQueries({ queryKey: keys.sessions.detail(session.id) })
      void queryClient.invalidateQueries({ queryKey: keys.sessions.list(bookId) })
    },
  })
}

export function useAnswerSession(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ sessionId, input }: { sessionId: number; input: SessionAnswerInput }) =>
      endpoints.answerSession(sessionId, input),
    onSuccess: (session) => {
      void queryClient.invalidateQueries({ queryKey: keys.sessions.detail(session.id) })
      void queryClient.invalidateQueries({ queryKey: keys.sessions.list(bookId) })
    },
  })
}

export function usePatchSession(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ sessionId, patch }: { sessionId: number; patch: SessionPatch }) =>
      endpoints.patchSession(sessionId, patch),
    onSuccess: (session) => {
      void queryClient.invalidateQueries({ queryKey: keys.sessions.detail(session.id) })
      void queryClient.invalidateQueries({ queryKey: keys.sessions.list(bookId) })
    },
  })
}

export function useStartDebugSession(bookId: number) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => endpoints.startDebugSession(bookId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.sessions.list(bookId) })
    },
  })
}
