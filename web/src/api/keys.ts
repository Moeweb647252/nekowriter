/** The one query-key factory (spec/11 §4.1). No ad-hoc arrays elsewhere. */

export const keys = {
  health: ["health"] as const,
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
    reviews: (id: number) => ["chapters", id, "reviews"] as const,
  },
  sessions: {
    list: (bookId: number, filters?: string) =>
      ["sessions", "list", bookId, filters ?? ""] as const,
    detail: (id: number) => ["sessions", "detail", id] as const,
  },
}

export type QueryKeys = typeof keys
