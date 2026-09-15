import { lazy, Suspense, type ReactNode } from "react"
import { Navigate, RouterProvider, createHashRouter } from "react-router"

import { ShortcutHelpPanel } from "@/components/ShortcutHelpPanel"

const DashboardPage = lazy(async () => {
  const page = await import("@/pages/DashboardPage")
  return { default: page.DashboardPage }
})
const KnowledgePage = lazy(async () => {
  const page = await import("@/pages/KnowledgePage")
  return { default: page.KnowledgePage }
})
const RecyclePage = lazy(async () => {
  const page = await import("@/pages/RecyclePage")
  return { default: page.RecyclePage }
})
const SettingsPage = lazy(async () => {
  const page = await import("@/pages/SettingsPage")
  return { default: page.SettingsPage }
})
const TemplatesPage = lazy(async () => {
  const page = await import("@/pages/TemplatesPage")
  return { default: page.TemplatesPage }
})
const WorksLibraryPage = lazy(async () => {
  const page = await import("@/pages/WorksLibraryPage")
  return { default: page.WorksLibraryPage }
})
const WorkspacePage = lazy(async () => {
  const page = await import("@/pages/WorkspacePage")
  return { default: page.WorkspacePage }
})

function RouteLoading() {
  return (
    <main className="grid min-h-svh place-items-center bg-background p-6 text-sm text-muted-foreground">
      正在打开页面…
    </main>
  )
}

function routeElement(element: ReactNode) {
  return <Suspense fallback={<RouteLoading />}>{element}</Suspense>
}

const router = createHashRouter([
  { index: true, element: <Navigate replace to="/works" /> },
  { path: "/works", element: routeElement(<WorksLibraryPage />) },
  {
    path: "/works/:projectId/versions/:versionId",
    element: routeElement(<WorkspacePage />),
  },
  { path: "/dashboard", element: routeElement(<DashboardPage />) },
  { path: "/knowledge", element: routeElement(<KnowledgePage />) },
  { path: "/templates", element: routeElement(<TemplatesPage />) },
  { path: "/settings", element: routeElement(<SettingsPage />) },
  { path: "/recycle", element: routeElement(<RecyclePage />) },
  { path: "*", element: <Navigate replace to="/works" /> },
])

export default function App() {
  return (
    <>
      <RouterProvider router={router} />
      <ShortcutHelpPanel />
    </>
  )
}
