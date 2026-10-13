import "i18next"

import type { EnglishCatalog, NAMESPACES } from "@/locales/en"

declare module "i18next" {
  interface CustomTypeOptions {
    // Every namespace is in scope by default, and every selector names its namespace:
    // `t(($) => $.library.title)`. The type checker follows selectors into the catalog.
    defaultNS: typeof NAMESPACES
    resources: EnglishCatalog
    enableSelector: "strict"
  }
}
