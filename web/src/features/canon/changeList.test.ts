/**
 * The change-log diff rendering (PROJECT §8.8, spec/02 §4).
 *
 * `snapshotText` turns a typed snapshot into the lines the Changes page shows. The rules that
 * matter: `type` and `track_id` are metadata, empty fields are skipped so a diff never shows a
 * blank line for an unset value, and arrays and attribute maps render inline.
 */
import { describe, expect, it } from "vitest"

import { snapshotText } from "@/features/canon/ChangeList"

describe("snapshotText", () => {
  it("renders the domain fields of an entity snapshot", () => {
    const text = snapshotText({
      type: "entity",
      kind: "character",
      name: "Lin Fan",
      aliases: ["Fan Ge"],
      summary: "an outer disciple",
      description: "",
      state: "in the sword tomb",
      attributes: { realm: "foundation" },
      importance: 5,
      status: "active",
      is_protagonist: true,
      track_id: null,
    })
    expect(text).toContain("name: Lin Fan")
    expect(text).toContain("aliases: Fan Ge")
    expect(text).toContain("attributes: realm=foundation")
    expect(text).toContain("is_protagonist: true")
    expect(text).not.toContain("type:")
    expect(text).not.toContain("track_id:")
  })

  it("skips empty fields so a diff shows no blank lines", () => {
    const text = snapshotText({
      type: "note",
      content: "The sword was a gift.",
      chapter_id: null,
      importance: 3,
      entity_ids: [],
      status: "active",
    })
    expect(text).toBe("content: The sword was a gift.\nimportance: 3\nstatus: active")
  })

  it("renders a timeline event's entity ids inline", () => {
    const text = snapshotText({
      type: "timeline_event",
      position: 1024,
      title: "The gate",
      entity_ids: [12, 15],
    })
    expect(text).toContain("entity_ids: 12, 15")
    expect(text).toContain("position: 1024")
  })

  it("renders an empty snapshot as an empty string", () => {
    expect(snapshotText({ type: "entity", track_id: null })).toBe("")
  })
})
