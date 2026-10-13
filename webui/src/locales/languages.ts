/**
 * The languages of the user interface. Each is named in its own language, so people can
 * find theirs whatever the current interface language is.
 */
export const UI_LANGUAGES = ["en", "zh-CN"] as const

export type UiLanguage = (typeof UI_LANGUAGES)[number]

export const UI_LANGUAGE_NAMES: Record<UiLanguage, string> = {
  en: "English",
  "zh-CN": "简体中文",
}

/** Maps a browser language tag to the interface language that serves it. */
export function uiLanguageFor(tag: string): UiLanguage {
  return tag.toLowerCase().startsWith("zh") ? "zh-CN" : "en"
}
