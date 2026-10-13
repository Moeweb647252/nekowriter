import type { ProjectView } from "@/api/generated/ProjectView"

export function projectView(overrides: Partial<ProjectView> = {}): ProjectView {
  return {
    id: "0190f5c8-0000-7000-8000-000000000001",
    name: "Rain",
    writingLanguage: "en",
    selectedVersion: {
      id: "0190f5c8-0000-7000-8000-000000000002",
      number: 1,
      novelTitle: "Rain Letters",
    },
    revision: 1,
    createdAt: 1_760_000_000_000,
    updatedAt: 1_760_000_000_000,
    deletedAt: null,
    ...overrides,
  }
}
