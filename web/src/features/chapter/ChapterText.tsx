/**
 * The reading column with in-place paragraph editing and a full-text editor (PROJECT §8.5).
 *
 * A held **chapter write** is different from the other holds: what the author decides on is the
 * document itself, so the pane renders the merged result (confirmed draft + staged content, see
 * `pendingWrite.ts`) and the decision lives in the dock that `ChapterPage` puts under this column
 * (D-141). While that merged view is up, the paragraphs are read-only: editing them would bump the
 * version and make the staged write stale, and the text on screen is not the draft's text. Every
 * other hold keeps the inline `HoldBar` under the paragraphs it touched.
 */
import { Fragment, memo, useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { AnimatePresence, motion } from "motion/react"

import { useContentMutations } from "@/api/hooks/chapters"
import { useResumeSession } from "@/api/hooks/sessions"
import { useHeldDecision } from "@/features/agent/useHeldDecision"
import { GhostText, HoldBar, type Preview } from "@/features/chapter/GhostText"
import { PendingText } from "@/features/chapter/PendingText"
import type { PendingDraftView } from "@/features/chapter/pendingWrite"
import { useWriterState } from "@/features/chapter/useWriterState"
import { useErrorToast } from "@/components/common/ErrorToast"
import { EmptyState } from "@/components/common/EmptyState"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { Lock } from "lucide-react"

import type { ChapterDto } from "@/api/gen/ChapterDto"
import type { ContentOpInput } from "@/api/gen/ContentOpInput"
import { useChapterDoc } from "@/stores/chapterDoc"
import { isComposing } from "@/lib/hotkeys"
import { paragraphKeyAction } from "@/lib/paragraphEditing"
import { countWords } from "@/lib/wordcount"
import { cn } from "@/lib/utils"

export type ChapterTextProps = {
  chapter: ChapterDto
  bookId: number
  unspaced: boolean
  fullTextOpen: boolean
  onFullTextOpenChange: (open: boolean) => void
  /** A pid to scroll to and highlight (a review issue jumped here). */
  focusPid?: string
  /**
   * The held write's merged view, when there is one that still applies. `ChapterPage` derives it
   * (it owns the dock that decides the call) and passes it in, so the pane and the dock cannot
   * disagree about what is on screen.
   */
  pendingView?: PendingDraftView
}

/** Renders the open document; a click edits one paragraph, the toolbar switches to full text. */
export function ChapterText({
  chapter,
  bookId,
  unspaced,
  fullTextOpen,
  onFullTextOpenChange,
  focusPid,
  pendingView,
}: ChapterTextProps) {
  const { t } = useTranslation()
  const showError = useErrorToast()
  const fullTextToken = useRef(0)
  const wasFullTextOpen = useRef(fullTextOpen)
  useEffect(() => {
    if (fullTextOpen && !wasFullTextOpen.current) {
      fullTextToken.current += 1
    }
    wasFullTextOpen.current = fullTextOpen
  }, [fullTextOpen])
  const paragraphs = useChapterDoc((state) => state.paragraphs)
  const tailOpen = useChapterDoc((state) => state.tailOpen)
  const version = useChapterDoc((state) => state.version)
  const nextPid = useChapterDoc((state) => state.nextPid)
  const highlights = useChapterDoc((state) => state.highlights)
  const clearHighlight = useChapterDoc((state) => state.clearHighlight)
  const { patch, putText } = useContentMutations(chapter.id, bookId)
  const resume = useResumeSession(bookId)
  const writer = useWriterState(chapter, undefined)
  const locked = writer.locked
  // The inline bar's two decisions go through the shared contract (D-141): approve sends no
  // feedback, request changes sends exactly the trimmed text and nothing at all when it is blank.
  const held = useHeldDecision(writer.session?.id ?? null, resume, showError)
  // A merged write preview supersedes the editable document: that text is not the draft's, so it
  // must not be edited here (the dock under the pane owns the decision instead).
  const preview = pendingView !== undefined
  const [pendingAfter, setPendingAfter] = useState<{
    pid: string
    text: string
    startText: string
  } | null>(null)

  // Stable per-render callbacks: each `Paragraph` is memoized on its own props (spec/11 §5.2), so
  // a streaming tail paragraph must not re-render the other hundreds.
  const patchMutate = patch.mutate
  const saveParagraph = useCallback(
    (pid: string, text: string, startText: string) => {
      // Paragraph staleness is per paragraph, not per document version (R9-01): a preceding save
      // of another paragraph may advance the version and is still a legal base for this edit.
      const current = paragraphs.find((paragraph) => paragraph.pid === pid)?.text
      if (current !== startText) {
        return
      }
      const ops: ContentOpInput[] = [{ op: "update", pid, text }]
      patchMutate({ base_version: version, ops }, { onError: showError })
    },
    [patchMutate, paragraphs, version, showError],
  )
  const insertParagraphAfter = useCallback(
    (pid: string, text: string, startText: string) => {
      const current = paragraphs.find((paragraph) => paragraph.pid === pid)?.text
      if (current !== startText) {
        return
      }
      // Enter only opens a pending editor; nothing is persisted until the author types and
      // leaves it. An empty pending paragraph therefore never enters the data model (R10-01).
      setPendingAfter({ pid, text, startText })
    },
    [paragraphs],
  )
  const commitPending = useCallback(
    (pid: string, text: string, startText: string, newText: string) => {
      const current = paragraphs.find((paragraph) => paragraph.pid === pid)?.text
      if (current !== startText) {
        return
      }
      const ops: ContentOpInput[] = []
      let anchor = pid
      if (text !== startText) {
        ops.push({ op: "update", pid, text })
        // An update splits embedded line breaks into new paragraphs. The pending paragraph
        // belongs after the last one, not before the split tail (R10-04).
        const lines = text.split("\n").filter((line) => line.trim() !== "").length
        if (lines > 1) {
          anchor = `p${nextPid + lines - 2}`
        }
      }
      ops.push({ op: "insert", after: anchor, text: newText })
      patchMutate({ base_version: version, ops }, {
        onError: showError,
        onSuccess: () => setPendingAfter(null),
      })
    },
    [nextPid, patchMutate, paragraphs, version, showError],
  )
  const dismissPending = useCallback(
    (pid: string, text: string, startText: string) => {
      const current = paragraphs.find((paragraph) => paragraph.pid === pid)?.text
      if (current === startText && text !== startText) {
        patchMutate(
          { base_version: version, ops: [{ op: "update", pid, text }] },
          { onError: showError, onSuccess: () => setPendingAfter(null) },
        )
      } else {
        setPendingAfter(null)
      }
    },
    [patchMutate, paragraphs, version, showError],
  )

  const deleteParagraph = useCallback(
    (pid: string, startText: string) => {
      const current = paragraphs.find((paragraph) => paragraph.pid === pid)?.text
      if (current !== startText) {
        return
      }
      const ops: ContentOpInput[] = [{ op: "delete", pid }]
      patchMutate({ base_version: version, ops }, { onError: showError })
    },
    [patchMutate, paragraphs, version, showError],
  )


  // The live count while typing; the server value is authoritative (spec/11 §8).
  const liveWords = useMemo(
    () => paragraphs.reduce((total, paragraph) => total + countWords(paragraph.text), 0),
    [paragraphs],
  )

  if (fullTextOpen) {
    return (
      <FullTextEditor
        key={chapter.id}
        chapter={chapter}
        version={version}
        locked={locked}
        saving={putText.isPending}
        savingParagraph={patch.isPending}
        onCancel={() => onFullTextOpenChange(false)}
        onSave={(text) => {
          const saveToken = fullTextToken.current
          putText.mutate(
            { base_version: version, text },
            {
              onSuccess: () => {
                // Do not close an editor the author reopened while this save was in flight (S-4).
                if (fullTextToken.current === saveToken) {
                  onFullTextOpenChange(false)
                }
              },
              onError: showError,
            },
          )
        }}
      />
    )
  }

  return (
    <div className="mx-auto max-w-[42em]">
      {locked ? (
        <div
          className="mb-3 flex items-center gap-2 rounded-md border border-status-running/40 bg-status-running/10 px-3 py-2 text-xs text-muted-foreground"
          role="status"
        >
          <Lock className="size-3.5 shrink-0" aria-hidden="true" />
          {t("chapter:locked_banner")}
        </div>
      ) : null}
      <div className="mb-4 flex items-center justify-between gap-2">
        {/*
          The count is stated for the document on screen, and named when that is not the confirmed
          draft: the header shows the server's confirmed count (spec/11 §8), so an unlabelled
          different number here would contradict it instead of agreeing with it.
        */}
        <p className="text-xs text-muted-foreground">
          {pendingView === undefined
            ? t("chapter:live_words", { words: liveWords })
            : t("chapter:pending_live_words", {
                staged: pendingView.stagedWords,
                confirmed: pendingView.confirmedWords,
              })}
        </p>
        <Button
          size="sm"
          variant="outline"
          // Editing the merged preview would edit text that is not the draft's, and the save would
          // bump the version the staged write is compared against.
          disabled={locked || preview}
          onClick={() => onFullTextOpenChange(true)}
        >
          {t("chapter:full_text_edit")}
        </Button>
      </div>
      {pendingView !== undefined ? (
        <PendingText paragraphs={pendingView.paragraphs} unspaced={unspaced} />
      ) : paragraphs.length === 0 ? (
        <EmptyState title={t("chapter:empty_title")} description={t("chapter:empty_text")} />
      ) : (
        <div className={unspaced ? "prose-cjk" : "prose-latin"}>
          <AnimatePresence initial={false}>
            {paragraphs.map((paragraph, index) => (
              <Fragment key={paragraph.pid}>
              <Paragraph
                key={paragraph.pid}
                pid={paragraph.pid}
                text={paragraph.text}
                // An `insert_text` preview streams into the open tail paragraph (spec/11 §5.2).
                ghost={
                  writer.preview?.name === "insert_text" &&
                  tailOpen &&
                  index === paragraphs.length - 1
                    ? writer.preview
                    : undefined
                }
                onlyParagraph={paragraphs.length === 1}
                focus={paragraph.pid === focusPid}
                highlight={highlights[paragraph.pid]?.kind}
                onHighlightEnd={clearHighlight}
                saving={patch.isPending}
                locked={locked}
                onSave={saveParagraph}
                onInsertAfter={insertParagraphAfter}
                onDelete={deleteParagraph}
              />
              {pendingAfter?.pid === paragraph.pid ? (
                <PendingParagraph
                  key={`pending-${paragraph.pid}`}
                  onPersist={(newText) =>
                    commitPending(paragraph.pid, pendingAfter.text, pendingAfter.startText, newText)
                  }
                  onDismiss={() =>
                    dismissPending(paragraph.pid, pendingAfter.text, pendingAfter.startText)
                  }
                />
              ) : null}
              </Fragment>
            ))}
          </AnimatePresence>
          {/* The paragraph tools' previews are a block of their own. */}
          {tailOpen ? null : <GhostText preview={writer.preview} />}
          {/*
            A held chapter write is decided in the dock under this pane (D-141), so the inline bar
            is only for the holds that are not about this document's text.
          */}
          {writer.held === undefined || writer.heldWrite ? null : (
            <HoldBar
              key={chapter.id}
              outline={writer.heldOutline}
              pids={writer.heldPids}
              busy={held.busy}
              onContinue={held.approve}
              onFeedback={held.requestChanges}
            />
          )}
        </div>
      )}
    </div>
  )
}

type ParagraphProps = {
  pid: string
  text: string
  /** The live preview to append inside this paragraph (the open tail), if any. */
  ghost?: Preview
  /** The document has exactly one paragraph, which may not be deleted. */
  onlyParagraph: boolean
  /** True when a review issue jumped to this paragraph. */
  focus?: boolean
  highlight?: "inserted" | "updated"
  saving: boolean
  locked: boolean
  onHighlightEnd: (pid: string) => void
  onSave: (pid: string, text: string, startText: string) => void
  onInsertAfter: (pid: string, text: string, startText: string) => void
  onDelete: (pid: string, startText: string) => void
}

/**
 * One paragraph: click to edit in place; Esc cancels; blur or ⌘⏎ saves; Enter at the end starts
 * the next paragraph and Backspace in an empty one deletes it (spec/11 §5.2).
 */
export const Paragraph = memo(function Paragraph({
  pid,
  text,
  ghost,
  onlyParagraph,
  focus,
  highlight,
  saving,
  locked,
  onHighlightEnd,
  onSave,
  onInsertAfter,
  onDelete,
}: ParagraphProps) {
  const { t } = useTranslation()
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(text)
  const [startText, setStartText] = useState(text)
  const [stale, setStale] = useState(false)

  useEffect(() => {
    if (focus === true) {
      document.querySelector(`[data-pid="${pid}"]`)?.scrollIntoView({ block: "center" })
    }
  }, [focus, pid])

  if (stale) {
    return (
      <motion.div layout className="my-1 space-y-1">
        <p className="whitespace-pre-wrap">{text}</p>
        <p role="status" className="text-xs text-destructive">
          {t("chapter:paragraph_stale")}
        </p>
        <Button
          size="sm"
          variant="outline"
          onClick={() => {
            setDraft(text)
            setStartText(text)
            setEditing(false)
            setStale(false)
          }}
        >
          {t("chapter:paragraph_stale_reload")}
        </Button>
      </motion.div>
    )
  }

  if (editing) {
    return (
      <motion.div layout className="my-1">
        <Textarea
          value={draft}
          autoFocus
          rows={Math.max(2, Math.ceil(draft.length / 40))}
          disabled={saving}
          className="resize-none"
          onChange={(event) => setDraft(event.target.value)}
          onBlur={() => {
            if (draft !== text) {
              if (text !== startText) {
                setEditing(false)
                setStale(true)
                return
              }
              onSave(pid, draft, startText)
            }
            setEditing(false)
          }}
          onKeyDown={(event) => {
            const caret = event.currentTarget.selectionStart ?? draft.length
            const action = paragraphKeyAction({
              key: event.key,
              composing: isComposing(event),
              caret,
              value: draft,
              primaryModifier: event.metaKey || event.ctrlKey,
              shiftKey: event.shiftKey,
            })
            if (action === "none") {
              return
            }
            event.preventDefault()
            if (action === "cancel") {
              setDraft(text)
              setEditing(false)
              return
            }
            if (action === "save") {
              if (text !== startText) {
                setEditing(false)
                setStale(true)
                return
              }
              setEditing(false)
              if (draft !== text) {
                onSave(pid, draft, startText)
              }
              return
            }
            if (action === "insert-after") {
              if (text !== startText) {
                setEditing(false)
                setStale(true)
                return
              }
              setEditing(false)
              onInsertAfter(pid, draft, startText)
              return
            }
            if (action === "delete-paragraph") {
              // Keep one paragraph: an empty document has no anchor for the author's next click,
              // and the full-text editor is the way to empty a chapter.
              if (onlyParagraph) {
                return
              }
              if (text !== startText) {
                setEditing(false)
                setStale(true)
                return
              }
              setEditing(false)
              onDelete(pid, startText)
            }
          }}
        />
      </motion.div>
    )
  }

  return (
    <motion.p
      layout
      data-pid={pid}
      initial={{ opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, height: 0 }}
      transition={{ duration: 0.18 }}
      title={pid}
      onClick={() => {
        if (locked) {
          return
        }
        setDraft(text)
        setStartText(text)
        setStale(false)
        setEditing(true)
      }}
      onAnimationComplete={() => {
        if (highlight !== undefined) {
          onHighlightEnd(pid)
        }
      }}
      className={cn(
        "cursor-text rounded-sm",
        highlight === "inserted" && "bg-status-done/10",
        highlight === "updated" && "bg-status-running/10",
        focus === true && "bg-amber-500/10 ring-2 ring-amber-500/40",
      )}
    >
      {text}
      {ghost === undefined ? null : (
        <span className="ghost-text" aria-live="polite">
          {ghost.text}
          <span className="ghost-caret" aria-hidden="true" />
        </span>
      )}
    </motion.p>
  )
})

export function PendingParagraph({
  onPersist,
  onDismiss,
}: {
  onPersist: (text: string) => void
  onDismiss: () => void
}) {
  const { t } = useTranslation()
  const [text, setText] = useState("")
  const submitted = useRef(false)
  const ref = useRef<HTMLTextAreaElement>(null)
  useEffect(() => {
    ref.current?.focus()
  }, [])
  const finish = () => {
    if (submitted.current) {
      return
    }
    submitted.current = true
    const value = text.trim()
    if (value === "") {
      onDismiss()
    } else {
      onPersist(value)
    }
  }
  return (
    <motion.div layout className="my-1">
      <Textarea
        ref={ref}
        value={text}
        autoFocus
        rows={2}
        aria-label={t("chapter:new_paragraph")}
        placeholder={t("chapter:new_paragraph_placeholder")}
        onChange={(event) => setText(event.target.value)}
        onBlur={finish}
        onKeyDown={(event) => {
          if (isComposing(event)) {
            return
          }
          if (event.key === "Escape") {
            event.preventDefault()
            onDismiss()
          }
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault()
            finish()
          }
        }}
      />
    </motion.div>
  )
}

type FullTextEditorProps = {
  chapter: ChapterDto
  version: number
  saving: boolean
  savingParagraph: boolean
  locked: boolean
  onCancel: () => void
  onSave: (text: string) => void
}

/** One textarea; paragraphs are separated by blank lines (spec/11 §5.2). */
function FullTextEditor({
  chapter,
  version,
  saving,
  savingParagraph,
  locked,
  onCancel,
  onSave,
}: FullTextEditorProps) {
  const { t } = useTranslation()
  const [text, setText] = useState(() => paragraphTextOf(chapter))
  const [dirty, setDirty] = useState(false)
  // The editor opens against one version. Prefer the live store when it already holds this
  // chapter (a paragraph blur may have advanced it); otherwise use the chapter DTO, which is
  // correct on the first render after a chapter switch before the load effect runs (R8-01).
  const storeChapterId = useChapterDoc((state) => state.chapterId)
  const storeVersion = useChapterDoc((state) => state.version)
  const storeParagraphs = useChapterDoc((state) => state.paragraphs)
  const [baseVersion, setBaseVersion] = useState(
    storeChapterId === chapter.id ? storeVersion : chapter.content_version,
  )
  const [openedDuringParagraphSave] = useState(savingParagraph)
  const [lastVersion, setLastVersion] = useState(version)
  if (
    openedDuringParagraphSave &&
    !dirty &&
    storeChapterId === chapter.id &&
    version !== lastVersion
  ) {
    // A paragraph blur saved just before this editor mounted; when its response advances the
    // store, rebase the untouched editor instead of freezing the stale cached chapter text
    // (R11-03).
    setLastVersion(version)
    setBaseVersion(version)
    setText(storeParagraphs.map((paragraph) => paragraph.text).join("\n\n"))
  }
  const stale = version !== baseVersion
  return (
    <div className="mx-auto max-w-[42em] space-y-3">
      <p className="text-xs text-muted-foreground">{t("chapter:full_text_hint")}</p>
      {locked ? (
        <p className="flex items-center gap-1 text-xs text-muted-foreground">
          <Lock className="size-3.5 shrink-0" aria-hidden="true" />
          {t("chapter:locked_banner")}
        </p>
      ) : null}
      {stale ? (
        <p role="status" className="text-xs text-destructive">
          {t("chapter:full_text_stale")}
        </p>
      ) : null}
      <Textarea
        value={text}
        rows={28}
        autoFocus
        disabled={locked || stale}
        className="font-serif text-[1.05rem] leading-relaxed"
        onChange={(event) => {
          setDirty(true)
          setText(event.target.value)
        }}
        onKeyDown={(event) => {
          if (isComposing(event)) {
            return
          }
          if (event.key === "Escape") {
            onCancel()
          }
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault()
            if (!stale) {
              onSave(text)
            }
          }
        }}
      />
      <div className="flex items-center gap-2">
        <Button size="sm" disabled={saving || locked || stale} onClick={() => onSave(text)}>
          {saving ? t("common:saving") : t("common:save")}
        </Button>
        <Button size="sm" variant="ghost" disabled={saving} onClick={onCancel}>
          {t("common:cancel")}
        </Button>
        <span className="ml-auto text-xs text-muted-foreground">
          {t("chapter:base_version", { version })}
        </span>
      </div>
    </div>
  )
}

/** The stored text of a chapter as one blank-line separated document. */
function paragraphTextOf(chapter: ChapterDto): string {
  return chapter.content.paragraphs.map((paragraph) => paragraph.text).join("\n\n")
}
