import { createBrowserRouter, type RouteObject } from "react-router"
import { RouterProvider } from "react-router/dom"
import { useTranslation } from "react-i18next"

import { AppShell } from "@/app/shell/AppShell"

export const routes: RouteObject[] = [
  {
    path: "/",
    element: <AppShell />,
    errorElement: <RouteError />,
    children: [
      {
        index: true,
        lazy: async () => {
          const module = await import("@/features/library/LibraryPage")
          return { Component: module.default }
        },
      },
      {
        // The global knowledge base (PROJECT §8.10, spec/10 §2.5). The scope and entry selection
        // is part of the URL (spec/11 §3), so all three shapes render the same page.
        path: "knowledge",
        lazy: async () => {
          const module = await import("@/features/knowledge/KnowledgePage")
          return { Component: module.default }
        },
      },
      {
        path: "knowledge/scopes/:scopeId",
        lazy: async () => {
          const module = await import("@/features/knowledge/KnowledgePage")
          return { Component: module.default }
        },
      },
      {
        path: "knowledge/scopes/:scopeId/entries/:entryId",
        lazy: async () => {
          const module = await import("@/features/knowledge/KnowledgePage")
          return { Component: module.default }
        },
      },
      {
        // The research task list and one task (PROJECT §8.10, spec/11 §5).
        path: "research",
        lazy: async () => {
          const module = await import("@/features/research/ResearchPage")
          return { Component: module.default }
        },
      },
      {
        path: "research/:taskId",
        lazy: async () => {
          const module = await import("@/features/research/ResearchTaskPage")
          return { Component: module.default }
        },
      },
      {
        // The global prompt pack (PROJECT §8.11, spec/11 §3).
        path: "prompts",
        lazy: async () => {
          const module = await import("@/features/prompts/PromptsPage")
          return { Component: module.default }
        },
      },
      {
        // The writing-skill catalogue (design §8).
        path: "skills",
        lazy: async () => {
          const module = await import("@/features/skills/SkillsPage")
          return { Component: module.default }
        },
      },
      {
        // Settings sections: providers, models, roles, appearance (PROJECT §8.12).
        path: "settings",
        lazy: async () => {
          const module = await import("@/features/settings/SettingsPage")
          return { Component: module.default }
        },
        children: [
          {
            index: true,
            lazy: async () => {
              const module = await import("@/features/settings/ProvidersPage")
              return { Component: module.default }
            },
          },
          {
            path: "providers",
            lazy: async () => {
              const module = await import("@/features/settings/ProvidersPage")
              return { Component: module.default }
            },
          },
          {
            path: "models",
            lazy: async () => {
              const module = await import("@/features/settings/ModelsPage")
              return { Component: module.default }
            },
          },
          {
            path: "roles",
            lazy: async () => {
              const module = await import("@/features/settings/RolesPage")
              return { Component: module.default }
            },
          },
          {
            path: "appearance",
            lazy: async () => {
              const module = await import("@/features/settings/AppearancePage")
              return { Component: module.default }
            },
          },
          {
            // The embedding model and the index state (spec/07 §4).
            path: "embeddings",
            lazy: async () => {
              const module = await import("@/features/settings/EmbeddingsPage")
              return { Component: module.default }
            },
          },
          {
            // The web search provider and its Test (spec/07 §6, spec/10 §2.1).
            path: "search",
            lazy: async () => {
              const module = await import("@/features/settings/WebSearchPage")
              return { Component: module.default }
            },
          },
        ],
      },
      {
        // The three-panel workspace (PROJECT §8.4, spec/11 §3).
        path: "books/:bookId",
        lazy: async () => {
          const module = await import("@/features/workspace/BookLayout")
          return { Component: module.default }
        },
        children: [
          {
            index: true,
            lazy: async () => {
              const module = await import("@/features/overview/OverviewPage")
              return { Component: module.default }
            },
          },
          {
            // The conception studio (PROJECT §8.7, spec/11 §5.4).
            path: "conception",
            lazy: async () => {
              const module = await import("@/features/conception/ConceptionStudio")
              return { Component: module.default }
            },
          },
          {
            path: "chapters/:chapterId",
            lazy: async () => {
              const module = await import("@/features/chapter/ChapterPage")
              return { Component: module.default }
            },
          },
          {
            path: "settings/:doc",
            lazy: async () => {
              const module = await import("@/features/settings-materials/SettingDocPage")
              return { Component: module.default }
            },
          },
          {
            path: "settings/characters",
            lazy: async () => {
              const module = await import("@/features/settings-materials/CharactersPage")
              return { Component: module.default }
            },
          },
          {
            // The canon pages (PROJECT §8.8, spec/10 §2.4).
            path: "canon/entities/:entityId?",
            lazy: async () => {
              const module = await import("@/features/canon/EntitiesPage")
              return { Component: module.default }
            },
          },
          {
            path: "canon/timeline",
            lazy: async () => {
              const module = await import("@/features/canon/TimelinePage")
              return { Component: module.default }
            },
          },
          {
            path: "canon/memory",
            lazy: async () => {
              const module = await import("@/features/canon/MemoryPage")
              return { Component: module.default }
            },
          },
          {
            path: "canon/foreshadowing",
            lazy: async () => {
              const module = await import("@/features/canon/ForeshadowingPage")
              return { Component: module.default }
            },
          },
          {
            path: "canon/goals",
            lazy: async () => {
              const module = await import("@/features/canon/GoalsPage")
              return { Component: module.default }
            },
          },
          {
            path: "canon/changes",
            lazy: async () => {
              const module = await import("@/features/canon/ChangesPage")
              return { Component: module.default }
            },
          },
          {
            // The archivist's setting proposals (PROJECT §8.5, spec/06 §5).
            path: "proposals",
            lazy: async () => {
              const module = await import("@/features/proposals/ProposalsPage")
              return { Component: module.default }
            },
          },
          {
            // The scopes this book's agents may search (PROJECT §8.10).
            path: "knowledge",
            lazy: async () => {
              const module = await import("@/features/knowledge/BookKnowledgePage")
              return { Component: module.default }
            },
          },
          {
            // The book-level prompt overrides (PROJECT §8.11).
            path: "prompts",
            lazy: async () => {
              const module = await import("@/features/prompts/BookPromptsPage")
              return { Component: module.default }
            },
          },
          {
            // A side story's premise, canon mode, anchor and chapters (PROJECT §8.4,
            // spec/11 §3 `tracks/:trackId`).
            path: "tracks/:trackId",
            lazy: async () => {
              const module = await import("@/features/workspace/SideStoryPage")
              return { Component: module.default }
            },
          },
          {
            path: "book-settings",
            lazy: async () => {
              const module = await import("@/features/settings-materials/BookSettingsPage")
              return { Component: module.default }
            },
          },
        ],
      },
    ],
  },
]

const router = createBrowserRouter(routes)

/**
 * The router's single error boundary (spec/11 §3). It is the fallback for a failed lazy-route
 * load, so it renders before any page component exists: it must go through `t()` like every other
 * user-visible string, or the one screen an author sees on a broken load is English in zh-CN.
 */
export function RouteError() {
  const { t } = useTranslation()
  return (
    <div className="flex min-h-svh items-center justify-center p-6">
      <div className="text-center">
        <h1 className="text-lg font-medium">{t("common:error_title")}</h1>
        <p className="mt-1 text-sm text-muted-foreground">{t("common:error_body")}</p>
        <button
          type="button"
          className="mt-3 text-sm underline"
          onClick={() => window.location.reload()}
        >
          {t("common:retry")}
        </button>
      </div>
    </div>
  )
}

export function AppRouter() {
  return <RouterProvider router={router} />
}
