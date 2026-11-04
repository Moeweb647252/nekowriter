/** Persisted UI preferences (spec/11 §4.3). */
import { create } from "zustand"
import { persist } from "zustand/middleware"

import { initialLanguage, type Language } from "@/i18n"

export type EditorFont = "serif" | "sans"
export type EditorSize = "s" | "m" | "l"
export type ContextView = "treemap" | "list"

type UiState = {
  editorFont: EditorFont
  editorSize: EditorSize
  navigatorCollapsed: boolean
  agentPanelCollapsed: boolean
  pinnedSession: Record<number, number | null>
  /**
   * Whether the pin of a book came from the session switcher. A manual pick always wins, even when
   * it is terminal; an automatic pin (a started session, an auto-followed one) yields to a newer
   * open session.
   */
  pinnedSessionManual: Record<number, boolean>
  /** A monotonically increasing request to open the book-agent panel for one book. */
  bookAgentRequest: Record<number, number>
  /** The last Context view choice (M23). */
  contextView: ContextView
  /** The UI language; persisted here and the source `i18n` restores at boot (spec/11 §4.3, §6). */
  language: Language
  setEditorFont: (font: EditorFont) => void
  setEditorSize: (size: EditorSize) => void
  setNavigatorCollapsed: (collapsed: boolean) => void
  setAgentPanelCollapsed: (collapsed: boolean) => void
  setContextView: (view: ContextView) => void
  setPinnedSession: (bookId: number, sessionId: number | null, manual?: boolean) => void
  requestBookAgent: (bookId: number) => void
  setLanguage: (language: Language) => void
}

export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      editorFont: "serif",
      editorSize: "m",
      navigatorCollapsed: false,
      agentPanelCollapsed: false,
      contextView: "treemap",
      pinnedSession: {},
      pinnedSessionManual: {},
      bookAgentRequest: {},
      language: initialLanguage(),
      setEditorFont: (editorFont) => set({ editorFont }),
      setEditorSize: (editorSize) => set({ editorSize }),
      setNavigatorCollapsed: (navigatorCollapsed) => set({ navigatorCollapsed }),
      setAgentPanelCollapsed: (agentPanelCollapsed) => set({ agentPanelCollapsed }),
      setContextView: (contextView) => set({ contextView }),
      // `manual` is explicit only for the session switcher; every other pin is automatic and may
      // later yield to a newer open session. Clearing the pin clears the flag with it.
      setPinnedSession: (bookId, sessionId, manual = false) =>
        set((state) => ({
          pinnedSession: { ...state.pinnedSession, [bookId]: sessionId },
          pinnedSessionManual: {
            ...state.pinnedSessionManual,
            [bookId]: sessionId !== null && manual,
          },
        })),
      requestBookAgent: (bookId) =>
        set((state) => ({
          bookAgentRequest: {
            ...state.bookAgentRequest,
            [bookId]: (state.bookAgentRequest[bookId] ?? 0) + 1,
          },
        })),
      setLanguage: (language) => set({ language }),
    }),
    { name: "nw.ui" },
  ),
)
