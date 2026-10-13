import type { EnglishCatalog } from "../en"
import type { Catalog } from "@/i18n/catalog"

import common from "./common"
import errors from "./errors"
import library from "./library"
import settings from "./settings"

/** The Simplified Chinese catalog. */
export const zhCN = { common, errors, library, settings } satisfies Catalog<EnglishCatalog>
