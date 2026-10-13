/**
 * Marks a branch the type checker proves unreachable, such as the default case of a
 * `switch` over every member of a union. Adding a member makes such a switch fail to
 * compile until the new member is handled.
 */
export function assertNever(value: never): never {
  throw new Error(`Unhandled value: ${JSON.stringify(value)}`)
}
