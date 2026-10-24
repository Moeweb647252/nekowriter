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
