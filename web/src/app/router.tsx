import { createBrowserRouter } from "react-router"
import { RouterProvider } from "react-router/dom"

import { AppShell } from "@/app/shell/AppShell"

const router = createBrowserRouter([
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
        // The global prompt pack (PROJECT §8.11, spec/11 §3).
        path: "prompts",
        lazy: async () => {
          const module = await import("@/features/prompts/PromptsPage")
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
            // The book-level prompt overrides (PROJECT §8.11).
            path: "prompts",
            lazy: async () => {
              const module = await import("@/features/prompts/BookPromptsPage")
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
])

function RouteError() {
  return (
    <div className="flex min-h-svh items-center justify-center p-6">
      <div className="text-center">
        <h1 className="text-lg font-medium">Something went wrong</h1>
        <button
          type="button"
          className="mt-3 text-sm underline"
          onClick={() => window.location.reload()}
        >
          Reload
        </button>
      </div>
    </div>
  )
}

export function AppRouter() {
  return <RouterProvider router={router} />
}
