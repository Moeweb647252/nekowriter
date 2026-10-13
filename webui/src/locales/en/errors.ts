import type { ErrorCatalog } from "@/i18n/errors"

export default {
  network: "NekoWriter could not be reached. Check that it is still running.",
  unexpected: "Something unexpected happened. Reload the page and try again.",
  reference: "Reference: {{requestId}}",
  field: {
    project_name: "Work name",
    novel_title: "Novel title",
  },
  resource: {
    project: "work",
    version: "version",
  },
  codes: {
    FIELD_INVALID: {
      empty: "{{field}} is required.",
      too_long: "{{field}} can be at most {{maxChars}} characters.",
      control_character: "{{field}} must be a single line.",
    },
    NOT_FOUND: "This {{resource}} no longer exists or has been deleted.",
    REVISION_CONFLICT:
      "This {{resource}} was changed elsewhere. The latest version is shown now; please try again.",
    IDEMPOTENCY_CONFLICT:
      "This action was already submitted with different content. Reload the page and check the result.",
    STATE_CONFLICT: {
      project_not_deleted: "This work is not deleted.",
    },
    MALFORMED_REQUEST: "The request was not understood. Reload the page and try again.",
    ROUTE_NOT_FOUND:
      "This page does not match the running NekoWriter. Reload the page to update it.",
    UNTRUSTED_HOST: "NekoWriter only answers requests addressed to this computer.",
    UNTRUSTED_ORIGIN: "NekoWriter only accepts changes from its own pages.",
    INTERNAL: "NekoWriter ran into an unexpected problem. The details are in its log.",
  },
} as const satisfies ErrorCatalog
