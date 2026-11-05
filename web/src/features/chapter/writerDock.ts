/**
 * The pure state table of the writer dock (PROJECT §6.4/§8.5, spec/11 §5.2, D-173).
 *
 * The component owns the input and the effects; this module owns the decisions, so every cell of
 * the auto-pause × status × text table can be tested without a browser mutation.
 */
import type { SessionDto } from "@/api/gen/SessionDto"
import type { ToolCallDto } from "@/api/gen/ToolCallDto"
import type { PendingStanding } from "@/features/chapter/pendingWrite"

export type WriterDockSurface = "dock" | "panel"
export type WriterDockAction =
  | "approve"
  | "feedback"
  | "feedback_keep"
  | "message"
  | "resume"
  | "pause_after_step"

export type WriterDockInput = {
  session: SessionDto | undefined
  held: ToolCallDto | undefined
  surface: WriterDockSurface | undefined
  standing: PendingStanding | undefined
  /** The dock's own step pause is pending or has landed as `paused`. */
  ownPause: boolean
  text: string
}

export type WriterDockState = {
  visible: boolean
  disabled: boolean
  button: "approve" | "send"
  buttonEnabled: boolean
  submit: WriterDockAction | null
  onFocus: "pause_after_step" | null
  onLeave: "resume" | null
  /** i18n key for the placeholder. */
  placeholder: string
  /** i18n key for the screen-reader description. */
  description: string
  /** A stable state name for `data-state` and tests. */
  state: string
}

const WRITER_KINDS = new Set(["chapter_write", "chapter_revise"])
const ACTIVE_PHASES = new Set(["drafting", "revising"])
const ACTIVE_STATUSES = new Set(["running", "held", "paused"])

function isWriter(session: SessionDto | undefined): boolean {
  return session !== undefined && WRITER_KINDS.has(session.kind)
}

function visible(session: SessionDto | undefined): boolean {
  return (
    isWriter(session) &&
    session !== undefined &&
    ACTIVE_PHASES.has(session.phase) &&
    ACTIVE_STATUSES.has(session.status)
  )
}

export function writerDockState(input: WriterDockInput): WriterDockState {
  const { session, held, surface, standing, ownPause } = input
  const text = input.text.trim()
  const hasText = text !== ""
  const panelHold = held !== undefined && surface === "panel"
  const dockHeld = held !== undefined && surface === "dock"
  const heldCompletes = held?.completes === true
  const off = session?.auto_pause === false
  const state = (name: string, values: Partial<WriterDockState>): WriterDockState => ({
    visible: true,
    disabled: false,
    button: "send",
    buttonEnabled: false,
    submit: null,
    onFocus: null,
    onLeave: null,
    placeholder: `chapter:dock.placeholder.${name}`,
    description: `chapter:dock.description.${name}`,
    state: name,
    ...values,
  })

  if (!visible(session)) {
    return { ...state("hidden", {}), visible: false }
  }

  if (panelHold) {
    // One decision surface (D-141/W14): the panel owns this hold.
    return state("panel_hold", { disabled: true, button: "approve" })
  }

  if (dockHeld) {
    const superseded = standing === "superseded"
    const base = superseded ? "superseded" : heldCompletes ? "held_completes" : "held"
    if (!off) {
      return state(base, {
        button: hasText ? "send" : "approve",
        buttonEnabled: true,
        submit: hasText ? "feedback" : "approve",
      })
    }
    return state(hasText ? "held_keep" : "held", {
      button: hasText ? "send" : "approve",
      buttonEnabled: true,
      submit: hasText ? "feedback_keep" : "approve",
    })
  }

  if (session?.status === "paused") {
    // Auto-pause on, or the dock's own pause: the empty button continues.
    if (!off || ownPause) {
      return state(ownPause ? "off_paused" : "on_paused", {
        button: hasText ? "send" : "approve",
        buttonEnabled: true,
        submit: hasText ? "feedback" : "resume",
        onLeave: ownPause && !hasText ? "resume" : null,
      })
    }
    return state("other_paused", {
      button: hasText ? "send" : "approve",
      buttonEnabled: true,
      submit: hasText ? "feedback" : "resume",
      onLeave: null,
    })
  }

  if (!off) {
    // Auto-pause on while running: steering only; approve waits for a hold or pause.
    return state("on_running", {
      button: hasText ? "send" : "approve",
      buttonEnabled: hasText,
      submit: hasText ? "message" : null,
    })
  }

  if (ownPause) {
    return state(hasText ? "off_pausing" : "off_pausing", {
      button: hasText ? "send" : "approve",
      buttonEnabled: hasText,
      submit: hasText ? "feedback" : null,
      onLeave: hasText ? null : "resume",
    })
  }
  if (session?.pause_requested === true) {
    return state("other_paused", {
      button: "send",
      buttonEnabled: hasText,
      submit: hasText ? "message" : null,
    })
  }
  return state("off_running", {
    button: "send",
    buttonEnabled: false,
    submit: hasText ? "feedback" : null,
    onFocus: "pause_after_step",
  })
}
