import { setupServer } from "msw/node"

/** The fake API for component tests. Each test installs the handlers it needs. */
export const server = setupServer()
