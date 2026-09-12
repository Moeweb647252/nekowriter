import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import "./index.css"
import App from "./App.tsx"
import { ThemeProvider } from "@/components/theme-provider.tsx"
import { ShortcutProvider } from "@/lib/shortcuts"

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ThemeProvider>
      <ShortcutProvider>
        <App />
      </ShortcutProvider>
    </ThemeProvider>
  </StrictMode>
)
