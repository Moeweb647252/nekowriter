/** The one query-key factory (spec/11 §4.1). No ad-hoc arrays elsewhere. */

export const keys = {
  health: ["health"] as const,
  appSettings: ["app-settings"] as const,
  providers: {
    all: ["providers"] as const,
    presets: ["provider-presets"] as const,
  },
  modelProfiles: {
    all: ["model-profiles"] as const,
  },
  prompts: {
    list: (bookId?: number) => ["prompts", "list", bookId ?? "global"] as const,
  },
  books: {
    all: ["books"] as const,
    list: (status?: string) => ["books", "list", status ?? "all"] as const,
    detail: (id: number) => ["books", "detail", id] as const,
    overview: (id: number) => ["books", "overview", id] as const,
    tracks: (id: number) => ["books", id, "tracks"] as const,
  },
  tracks: {
    detail: (id: number) => ["tracks", id] as const,
    chapters: (id: number) => ["tracks", id, "chapters"] as const,
  },
  chapters: {
    detail: (id: number) => ["chapters", "detail", id] as const,
    versions: (id: number) => ["chapters", id, "versions"] as const,
    version: (versionId: number) => ["chapters", "version", versionId] as const,
    reviews: (id: number) => ["chapters", id, "reviews"] as const,
  },
  settings: {
    docs: (bookId: number) => ["settings", "docs", bookId] as const,
    docHistory: (bookId: number, kind: string) =>
      ["settings", "docs", bookId, kind, "history"] as const,
    characters: (bookId: number) => ["settings", "characters", bookId] as const,
  },
  changes: {
    all: ["changes"] as const,
    list: (bookId: number, filter?: string) =>
      ["changes", "list", bookId, filter ?? "all"] as const,
  },
  sessions: {
    list: (bookId: number, filters?: string) =>
      ["sessions", "list", bookId, filters ?? ""] as const,
    detail: (id: number) => ["sessions", "detail", id] as const,
  },
}

export type QueryKeys = typeof keys
