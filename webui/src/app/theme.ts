/** Color themes. `system` follows the operating system's light or dark preference. */
export const THEMES = ["system", "light", "dark"] as const

export type Theme = (typeof THEMES)[number]

/** Where the chosen theme is remembered in this browser. */
export const THEME_STORAGE_KEY = "nekowriter.theme"
