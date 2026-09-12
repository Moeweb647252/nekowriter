# AGENTS.md

- Do not preserve backward compatibility. Remove obsolete paths instead of adding compatibility layers, fallbacks, backfills, or migrations.
- Choose the simplest implementation that fully meets the current requirements. Avoid speculative abstractions, configuration, and indirection.
- Grow the system in layers. Start from the smallest version that works end to end, and add each new capability on top of a product that already works. Never trade a working product for unfinished complexity.
- Do not plan any fallback strategy unless explicitly requested by user.
- Keep components modular and concerns clearly separated.
- Prefer established, well-maintained libraries when they reduce overall complexity or improve reliability. Do not reimplement common functionality without a clear reason.
- Lean on the dependencies already in the project before writing your own implementation or adding packages. Do not assume a library lacks a capability without checking its documentation and types.
- Make architectural decisions for the long term. Do not accept a stopgap that only works for now and is meant to be replaced later.
- Instead of quick, crude, and temporary, make implementations simple, robust and fully considered.
- Search with ripgrep (`rg`, or the editor's `code_search` / `glob` tools) instead of `grep`. Read a file in windows around a match rather than reading it whole.
- Do not compile or test just to look at something. Batch the verification a change actually needs into one run at the end of the change; a type check, a test run and a build are one verification, not three.
- The work of writing documents needs to be taken seriously. Make sure that the documents are standardized, clear, and accurate. The only one purpose of writing documents is to give the maintainer enough information.