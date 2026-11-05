import { useCallback, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { ArrowUp, Check, Loader2 } from "lucide-react"

import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import { keys } from "@/api/keys"
import { useQueryClient } from "@tanstack/react-query"
import { useResumeSession, useSendSessionMessage, useSessionControl } from "@/api/hooks/sessions"
import { useErrorToast } from "@/components/common/ErrorToast"
import { Button } from "@/components/ui/button"
import { heldDecisionSurface } from "@/features/agent/heldCall"
import type { PendingDraftView } from "@/features/chapter/pendingWrite"
import { useDraftRegionFocus } from "@/features/chapter/useDraftRegionFocus"
import { writerDockState, type WriterDockAction } from "@/features/chapter/writerDock"
import { isComposing } from "@/lib/hotkeys"

type Props = {
  bookId: number
  chapterId: number
  session: SessionDto | undefined
  held: ToolCallDto | undefined
  view: PendingDraftView | undefined
}

/** The one input under the draft (PROJECT §6.4/§8.5, spec/11 §5.2, D-173). */
export function WriterDock({ bookId, chapterId, session, held, view }: Props) {
  const { t } = useTranslation()
  const reportError = useErrorToast()
  const queryClient = useQueryClient()
  const resume = useResumeSession(bookId)
  const control = useSessionControl(bookId)
  const message = useSendSessionMessage(bookId)
  const [text, setText] = useState("")
  const [ownPause, setOwnPause] = useState(false)
  const [busy, setBusy] = useState(false)
  const queue = useRef<Promise<unknown>>(Promise.resolve())
  const generation = useRef(0)
  const textarea = useRef<HTMLTextAreaElement | null>(null)

  // A pause from an earlier status is no longer the dock's own; derive it rather than clearing
  // state in an effect (the repo's React Compiler rules).
  const activeOwnPause =
    ownPause && (session?.status === "running" || session?.status === "paused")
  const dock = writerDockState({
    session,
    held,
    surface: heldDecisionSurface(held),
    standing: view?.standing,
    ownPause: activeOwnPause,
    text,
  })

  const waitForSaves = useCallback(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
    while (queryClient.isMutating({ mutationKey: keys.chapters.contentWrite(chapterId) }) > 0) {
      await new Promise((resolve) => setTimeout(resolve, 25))
    }
  }, [chapterId, queryClient])

  const enqueue = useCallback((task: () => Promise<void>) => {
    setBusy(true)
    const next = queue.current.then(async () => {
      try {
        await task()
      } catch (error) {
        reportError(error)
      }
    })
    queue.current = next
    void next.finally(() => setBusy(false))
    return next
  }, [reportError])

  const submit = (action: WriterDockAction | null) => {
    if (session === undefined || action === null) return
    const feedback = text.trim()
    const afterSend = () => {
      setText("")
      textarea.current?.blur()
    }
    const sendResume = (keepWrite: boolean) => {
      if (feedback === "") return
      generation.current += 1
      enqueue(async () => {
        await resume.mutateAsync({
          sessionId: session.id,
          feedback,
          keep_write: keepWrite,
        })
        afterSend()
      })
    }
    switch (action) {
      case "approve":
        generation.current += 1
        enqueue(async () => {
          await resume.mutateAsync({ sessionId: session.id, keep_write: false })
          setOwnPause(false)
        })
        return
      case "resume":
        generation.current += 1
        enqueue(async () => {
          await resume.mutateAsync({ sessionId: session.id, keep_write: false })
          setOwnPause(false)
        })
        return
      case "feedback":
        sendResume(false)
        return
      case "feedback_keep":
        sendResume(true)
        return
      case "message":
        if (feedback === "") return
        generation.current += 1
        enqueue(async () => {
          await message.mutateAsync({ sessionId: session.id, text: feedback })
          afterSend()
        })
        return
      case "pause_after_step":
        return
    }
  }

  const onFocus = () => {
    if (dock.onFocus !== "pause_after_step" || session === undefined) return
    // A leave/resume queued before this focus must not run after the author re-paused (the
    // serial queue treats the new focus as a new generation).
    generation.current += 1
    setOwnPause(true)
    enqueue(async () => {
      await control.mutateAsync({ sessionId: session.id, operation: "pause", body: { after_step: true } })
    })
  }

  const onLeave = () => {
    if (dock.onLeave !== "resume" || session === undefined || text.trim() !== "") return
    const expected = ++generation.current
    enqueue(async () => {
      await waitForSaves()
      if (expected !== generation.current) return
      await resume.mutateAsync({ sessionId: session.id, keep_write: false })
      setOwnPause(false)
    })
  }

  const regionRef = useDraftRegionFocus(dock.onLeave !== null, onLeave)
  if (!dock.visible) return null

  const label = dock.button === "approve" ? t("chapter:dock.approve") : t("chapter:dock.send")
  const disabled = dock.disabled || !dock.buttonEnabled || busy

  return (
    <section
      ref={regionRef}
      data-testid="writer-dock"
      data-state={dock.state}
      className="shrink-0 border-t bg-background px-6 py-3"
    >
      <div className="mx-auto flex max-w-[42em] items-end gap-2">
        <textarea
          ref={textarea}
          aria-label={t("chapter:dock.input_label")}
          aria-describedby="writer-dock-description"
          rows={1}
          value={text}
          placeholder={t(dock.placeholder, {
            expected: view?.expectedVersion ?? 0,
            draft: view?.draftVersion ?? 0,
          })}
          disabled={dock.disabled}
          onFocus={onFocus}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => {
            if (isComposing(event)) return
            if (event.key === "Escape") {
              event.preventDefault()
              textarea.current?.blur()
              return
            }
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault()
              submit(dock.submit)
            }
          }}
          className="max-h-36 min-h-9 flex-1 resize-none rounded-2xl border bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
        />
        <Button
          type="button"
          size="icon"
          aria-label={label}
          aria-disabled={disabled}
          disabled={disabled}
          onClick={() => submit(dock.submit)}
          className="size-9 shrink-0 rounded-full"
        >
          {busy ? <Loader2 aria-hidden="true" className="animate-spin" /> : dock.button === "approve" ? <Check aria-hidden="true" /> : <ArrowUp aria-hidden="true" />}
        </Button>
      </div>
      <p id="writer-dock-description" className="sr-only">
        {t(dock.description)}
      </p>
    </section>
  )
}
