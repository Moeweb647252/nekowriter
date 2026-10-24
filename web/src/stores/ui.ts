/** Persisted UI preferences (spec/11 §4.3). */
import { create } from "zustand"
import { persist } from "zustand/middleware"

import { initialLanguage, type Language } from "@/i18n"

export type EditorFont = "serif" | "sans"
export type EditorSize = "s" | "m" | "l"

type UiState = {
  editorFont: EditorFont
  editorSize: EditorSize
  navigatorCollapsed: boolean
  agentPanelCollapsed: boolean
  pinnedSession: Record<number, number | null>
  /** The UI language; persisted here and the source `i18n` restores at boot (spec/11 §4.3, §6). */
  language: Language
  setEditorFont: (font: EditorFont) => void
  setEditorSize: (size: EditorSize) => void
  setNavigatorCollapsed: (collapsed: boolean) => void
  setAgentPanelCollapsed: (collapsed: boolean) => void
  setPinnedSession: (bookId: number, sessionId: number | null) => void
  setLanguage: (language: Language) => void
}

export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      editorFont: "serif",
      editorSize: "m",
      navigatorCollapsed: false,
      agentPanelCollapsed: false,
      pinnedSession: {},
      language: initialLanguage(),
      setEditorFont: (editorFont) => set({ editorFont }),
      setEditorSize: (editorSize) => set({ editorSize }),
      setNavigatorCollapsed: (navigatorCollapsed) => set({ navigatorCollapsed }),
      setAgentPanelCollapsed: (agentPanelCollapsed) => set({ agentPanelCollapsed }),
      setPinnedSession: (bookId, sessionId) =>
        set((state) => ({ pinnedSession: { ...state.pinnedSession, [bookId]: sessionId } })),
      setLanguage: (language) => set({ language }),
    }),
    { name: "nw.ui" },
  ),
)
