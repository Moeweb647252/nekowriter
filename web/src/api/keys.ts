/** The one query-key factory (spec/11 §4.1). No ad-hoc arrays elsewhere. */

export const keys = {
  health: ["health"] as const,
  appSettings: ["app-settings"] as const,
  providers: {
    all: ["providers"] as const,
    presets: ["provider-presets"] as const,
    optionSpecs: ["provider-presets", "options"] as const,
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
  canon: {
    /** The whole canon family, for `canon.changed` (spec/10 §3). */
    all: ["canon"] as const,
    entities: (bookId: number, filters?: string) =>
      ["canon", "entities", bookId, filters ?? "all"] as const,
    entity: (entityId: number) => ["canon", "entity", entityId] as const,
    relations: (bookId: number, entityId?: number) =>
      ["canon", "relations", bookId, entityId ?? "all"] as const,
    timeline: (bookId: number, trackId?: number) =>
      ["canon", "timeline", bookId, trackId ?? "all"] as const,
    notes: (bookId: number, filters?: string) =>
      ["canon", "notes", bookId, filters ?? "all"] as const,
    foreshadowing: (bookId: number, status?: string) =>
      ["canon", "foreshadowing", bookId, status ?? "all"] as const,
    memory: (bookId: number, position?: string) =>
      ["canon", "memory", bookId, position ?? "default"] as const,
    counts: (bookId: number) => ["canon", "counts", bookId] as const,
    /** Every goal list, invalidated by chapter.approved readiness changes and goal writes. */
    goalsAll: ["canon", "goals"] as const,
    goals: (bookId: number, filter?: string) =>
      ["canon", "goals", bookId, filter ?? "all"] as const,
    appearances: (chapterId: number) =>
      ["canon", "appearances", chapterId] as const,
  },
  knowledge: {
    all: ["knowledge"] as const,
    scopes: () => ["knowledge", "scopes"] as const,
    entries: (scopeId: number, q?: string) =>
      ["knowledge", "entries", scopeId, q ?? ""] as const,
    entry: (entryId: number) => ["knowledge", "entry", entryId] as const,
    search: (q: string, scopeIds: number[]) =>
      ["knowledge", "search", q, scopeIds.join(",")] as const,
    bookScopes: (bookId: number) => ["knowledge", "book-scopes", bookId] as const,
  },
  embedding: {
    status: ["embedding", "status"] as const,
  },
  research: {
    /** The whole research family, for `research.updated` (spec/10 §3). */
    all: ["research"] as const,
    list: (bookId?: number) => ["research", "list", bookId ?? "all"] as const,
    detail: (taskId: number) => ["research", "detail", taskId] as const,
  },
  proposals: {
    /** Every proposal list, for `proposal.changed` (spec/10 §3). */
    all: ["proposals"] as const,
    list: (bookId: number, status?: string) =>
      ["proposals", "list", bookId, status ?? "all"] as const,
  },
  skills: {
    /** Every writing-skill view, for any skill or selection write. */
    all: ["skills"] as const,
    catalogue: (bookId?: number) => ["skills", "catalogue", bookId ?? "global"] as const,
  },
  sessions: {
    list: (bookId: number, filters?: string) =>
      ["sessions", "list", bookId, filters ?? ""] as const,
    detail: (id: number) => ["sessions", "detail", id] as const,
  },
}

export type QueryKeys = typeof keys
