import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import "@/i18n"
import "./index.css"

import { App } from "@/app/App"

const root = document.getElementById("root")
if (!root) throw new Error("index.html has no #root element")

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
