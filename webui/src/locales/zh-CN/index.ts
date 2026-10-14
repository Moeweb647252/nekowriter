import type { EnglishCatalog } from "../en"
import type { Catalog } from "@/i18n/catalog"

import common from "./common"
import dashboard from "./dashboard"
import errors from "./errors"
import library from "./library"
import prompts from "./prompts"
import settings from "./settings"
import templates from "./templates"
import workspace from "./workspace"

/** The Simplified Chinese catalog. */
export const zhCN = {
  common,
  dashboard,
  errors,
  library,
  prompts,
  settings,
  templates,
  workspace,
} satisfies Catalog<EnglishCatalog>
