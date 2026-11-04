/** A small window event that opens the book-agent panel without coupling the navigator to the
 * store/i18n import order. */
const EVENT = "nw:book-agent-request"

export function requestBookAgent(bookId: number): void {
  if (typeof window !== "undefined") {
    window.dispatchEvent(new CustomEvent<number>(EVENT, { detail: bookId }))
  }
}

export function onBookAgentRequest(handler: (bookId: number) => void): () => void {
  const listener = (event: Event) => handler((event as CustomEvent<number>).detail)
  window.addEventListener(EVENT, listener)
  return () => window.removeEventListener(EVENT, listener)
}
