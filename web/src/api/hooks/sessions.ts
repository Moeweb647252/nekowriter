/** Session queries and mutations (spec/11 §4.1, §5.3). */
import {
  keepPreviousData,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query"

import * as endpoints from "@/api/endpoints/sessions"
import { keys } from "@/api/keys"
import type { MessageWithCallsDto } from "@/api/gen/MessageWithCallsDto"
import type { ResumeInput } from "@/api/gen/ResumeInput"
import type { SessionAnswerInput } from "@/api/gen/SessionAnswerInput"
import type { SessionMessageInput } from "@/api/gen/SessionMessageInput"
import type { SessionPatch } from "@/api/gen/SessionPatch"

/** Invalidates the session snapshot, the book list and the chapter that embeds `open_sessions`. */
function invalidateSession(
  queryClient: ReturnType<typeof useQueryClient>,
  bookId: number,
  chapterId: number | null | undefined,
): void {
  void queryClient.invalidateQueries({ queryKey: keys.sessions.list(bookId) })
  if (chapterId !== null && chapterId !== undefined && chapterId > 0) {
    // `ChapterDto.open_sessions` (spec/10 §2.8) is what the panel reads for a chapter.
    void queryClient.invalidateQueries({ queryKey: keys.chapters.detail(chapterId) })
  }
}

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

/**
 * Scrollback: older messages by cursor (spec/11 §5.3).
 *
 * The first page starts at the snapshot's `next_cursor` and later pages continue from each
 * response's own cursor. It is an on-demand query: `enabled` stays false, and the stream calls
 * `fetchNextPage` from the top button or the near-top scroll trigger, so opening a session never
 * fetches history the author has not asked for. Switching sessions changes the key (the session
 * id), which drops the loaded pages from the view.
 *
 * The page chain is anchored at the snapshot cursor that existed when its first page was
 * fetched. If a live message later makes the snapshot's 200-message window slide, the messages
 * between that anchor and the new cursor are in neither the cached pages nor the new snapshot;
 * `gapItems` re-fetches that range so no row disappears from the transcript.
 */
export function useOlderMessages(sessionId: number | null, initialCursor: string | undefined) {
  const pages = useInfiniteQuery({
    queryKey: [...keys.sessions.detail(sessionId ?? 0), "messages"] as const,
    queryFn: ({ pageParam }) => endpoints.getMessages(sessionId ?? 0, pageParam),
    initialPageParam: initialCursor,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: false,
  })
  const anchorCursor = pages.data?.pageParams[0] as string | undefined
  const gap = useQuery({
    queryKey: [
      ...keys.sessions.detail(sessionId ?? 0),
      "messages-gap",
      anchorCursor ?? "start",
      initialCursor ?? "end",
    ] as const,
    queryFn: async (): Promise<MessageWithCallsDto[]> => {
      if (anchorCursor === undefined || initialCursor === undefined) {
        return []
      }
      const anchorSeq = Number(anchorCursor)
      const items: MessageWithCallsDto[] = []
      let cursor: string | undefined = initialCursor
      // The endpoint caps a page at 500; walk backwards until the anchor is crossed so a large
      // window slide (offline reconnect, a burst of messages) still closes the whole gap.
      while (cursor !== undefined) {
        const page = await endpoints.getMessages(sessionId ?? 0, cursor, 500)
        for (const item of page.items) {
          if (item.message.seq >= anchorSeq) {
            items.push(item)
          }
        }
        const next = page.next_cursor ?? undefined
        if (next === undefined || Number(next) <= anchorSeq) {
          break
        }
        cursor = next
      }
      items.sort((left, right) => left.message.seq - right.message.seq)
      return items
    },
    enabled:
      sessionId !== null &&
      sessionId > 0 &&
      anchorCursor !== undefined &&
      initialCursor !== undefined &&
      anchorCursor !== initialCursor,
    placeholderData: keepPreviousData,
  })
  return { ...pages, gapItems: gap.data ?? [] }
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
      invalidateSession(queryClient, bookId, session.chapter_id)
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
      invalidateSession(queryClient, bookId, session.chapter_id)
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
      invalidateSession(queryClient, bookId, session.chapter_id)
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
      invalidateSession(queryClient, bookId, session.chapter_id)
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
      invalidateSession(queryClient, bookId, session.chapter_id)
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
