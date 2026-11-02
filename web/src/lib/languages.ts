/**
 * The writing languages the UI offers (PROJECT §8.3, §8.10); the API accepts any BCP-47 tag, so
 * this list is a convenience, not a constraint. Labels are translated through
 * `common:language_code.<code>` in both locales.
 */
export const WRITING_LANGUAGES = [
  "zh-CN",
  "en",
  "ja",
  "ko",
  "fr",
  "de",
  "es",
  "ru",
] as const
