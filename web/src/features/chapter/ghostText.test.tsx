/** Ghost text placement (spec/11 §5.2, spec/03 §12). */
import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { GhostText } from "@/features/chapter/GhostText"

describe("GhostText", () => {
  it("renders an insert_text preview", () => {
    render(<GhostText preview={{ name: "insert_text", field: "text", text: "他抬起头。" }} />)
    expect(screen.getByText("他抬起头。")).toBeInTheDocument()
  })

  it("renders an insert_paragraphs preview", () => {
    render(
      <GhostText preview={{ name: "insert_paragraphs", field: "text", text: "新的一段。" }} />,
    )
    expect(screen.getByText("新的一段。")).toBeInTheDocument()
  })

  it("renders a rewrite_paragraph preview", () => {
    render(
      <GhostText preview={{ name: "rewrite_paragraph", field: "text", text: "改写的一段。" }} />,
    )
    expect(screen.getByText("改写的一段。")).toBeInTheDocument()
  })

  it("renders nothing without a preview", () => {
    const { container } = render(<GhostText preview={undefined} />)
    expect(container).toBeEmptyDOMElement()
  })

  it("pins a handed-off writer preview without a caret", () => {
    const { container } = render(
      <GhostText
        preview={{ name: "insert_text", field: "text", text: "已写完的一句。", pinned: true }}
      />,
    )
    expect(container.querySelector(".ghost-text")?.textContent).toContain("已写完的一句。")
    expect(container.querySelector(".ghost-caret")).not.toBeInTheDocument()
  })

  it("renders nothing for a tool without a text preview field", () => {
    const { container } = render(
      <GhostText preview={{ name: "submit_outline", field: "outline", text: "场景一" }} />,
    )
    expect(container).toBeEmptyDOMElement()
  })
})

describe("GhostText placement", () => {
  it("renders an appended insert_text preview as an inline span", () => {
    const { container } = render(
      <GhostText preview={{ name: "insert_text", field: "text", text: "接着写。" }} appended />,
    )
    // Inside the open tail paragraph, so it must not introduce a block of its own.
    expect(container.querySelector("p")).toBeNull()
    expect(screen.getByText("接着写。")).toBeInTheDocument()
  })

  it("renders a block preview as a paragraph", () => {
    const { container } = render(
      <GhostText preview={{ name: "insert_text", field: "text", text: "新的一段。" }} />,
    )
    expect(container.querySelector("p.ghost-text")).not.toBeNull()
  })
})
