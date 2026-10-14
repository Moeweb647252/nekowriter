import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { ThemeProvider } from "next-themes"
import { useState } from "react"
import { BrowserRouter, Navigate, Route, Routes } from "react-router"

import { AppShell } from "./AppShell"
import { Splash } from "./Splash"
import { THEMES, THEME_STORAGE_KEY } from "./theme"
import { EventStreamProvider } from "@/api/events/EventStreamProvider"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { DashboardPage } from "@/features/dashboard/DashboardPage"
import { WorksLibraryPage } from "@/features/library/WorksLibraryPage"
import { PromptsPage } from "@/features/prompts/PromptsPage"
import { SettingsPage } from "@/features/settings/SettingsPage"
import { WorkspacePage } from "@/features/workspace/WorkspacePage"

/**
 * Cached reads never go stale on a timer: the event stream invalidates exactly what
 * changes. Failed reads are shown with a retry action instead of being retried silently.
 */
function createQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { staleTime: Number.POSITIVE_INFINITY, retry: false, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  })
}

export function App() {
  const [queryClient] = useState(createQueryClient)
  return (
    <ThemeProvider
      attribute="class"
      themes={[...THEMES]}
      defaultTheme="system"
      enableSystem
      disableTransitionOnChange
      storageKey={THEME_STORAGE_KEY}
    >
      <QueryClientProvider client={queryClient}>
        <TooltipProvider>
          <BrowserRouter>
            <EventStreamProvider splash={<Splash />}>
              <Routes>
                {/* The workspace has its own frame, without the page navigation. */}
                <Route path="/works/:projectId/versions/:versionId" element={<WorkspacePage />} />
                <Route element={<AppShell />}>
                  <Route path="/works" element={<WorksLibraryPage />} />
                  <Route path="/dashboard" element={<DashboardPage />} />
                  <Route path="/prompts" element={<PromptsPage />} />
                  <Route path="/settings" element={<SettingsPage />} />
                  <Route path="*" element={<Navigate to="/works" replace />} />
                </Route>
              </Routes>
            </EventStreamProvider>
          </BrowserRouter>
          <Toaster />
        </TooltipProvider>
      </QueryClientProvider>
    </ThemeProvider>
  )
}
