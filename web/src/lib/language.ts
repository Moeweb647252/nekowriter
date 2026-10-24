/**
 * Language helpers shared by the reading column and the diff view (spec/03 §8, spec/11 §5.2).
 *
 * The server owns the authoritative rules; this mirrors them so the UI can pick the prose style
 * and the diff granularity without a round trip.
 */

/** Languages written without spaces between words (spec/03 §8). */
const UNSPACED = ["zh", "ja", "th", "lo", "km", "my"]

/** The first subtag of a BCP-47 tag, lowercased. */
export function primarySubtag(language: string): string {
  return (language.split(/[-_]/)[0] ?? "").toLowerCase()
}

/** True when the writing language does not separate words with spaces. */
export function isUnspaced(language: string): boolean {
  return UNSPACED.includes(primarySubtag(language))
}
