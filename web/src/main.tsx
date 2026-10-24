import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import "./index.css"
import "@/i18n"
import { AppRouter } from "@/app/router"
import { Providers } from "@/app/providers"

const root = document.getElementById("root")
if (!root) {
  throw new Error("the root element is missing")
}

createRoot(root).render(
  <StrictMode>
    <Providers>
      <AppRouter />
    </Providers>
  </StrictMode>,
)
