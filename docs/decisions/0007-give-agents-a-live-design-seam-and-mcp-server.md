---
status: accepted
date: 2026-07-11
recorded: 2026-10-27
decision-makers: Jean Rojas
supersedes: [0005]
tags: [agents, contracts]
---

# Give agents a live design seam and an MCP server

## Context

Editing files alone ([0005](0005-let-agents-edit-designs-through-project-files.md))
left agents blind: they could not see the selection, render a page or apply a
typed edit. The obstacle had been the dependency direction, because Zed's
`agent` crate must not depend on the editor.

## Decision

Add a small seam crate, `design_surface`. It defines the `DesignSurface`
trait and typed ops and queries. `fig_viewer` registers its implementation as
a GPUI global, and the built-in agent's tools resolve it on each call. The
same surface is served to external agents over MCP by `fig_viewer`'s
`live_mcp`, reachable through `fanta --mcp-stdio`. Ops and queries are DTOs,
so tool schemas stay stable while the document model changes.

On 2026-10-21 the DTOs moved to `fanta-design-ops`, with no GPUI dependency,
so the headless engine applies the same ops (#155).

## Options considered

1. **A seam crate with a trait and DTOs** (chosen).
2. **Let the agent crate depend on `fig_viewer`.** The wrong direction for an
   inherited crate, and it would tie agent tools to editor internals.
3. **Files only.** See 0005.

## Consequences

- Good: agents inspect state, take screenshots and apply typed, undoable
  edits, both inside the app and from Claude Code or Codex over MCP.
- Bad: agent-facing descriptions now live in five places that nothing checks
  against each other (`crates/design_surface/AGENTS.md` lists them). Tool calls
  can arrive while the canvas entity is being updated.

## Confirmation

The layer map lets the shell reach the editor only through `design_surface`.
The `test_fanta_specs` and `test_design_progress` agent tests gate CI, and
`script/smoke-mcp` drives the loop against a running app.

## References

- Commits `317fb6622b` "design_surface: Add the agent/editor design seam" and `fff0ead336` "live MCP server exposing the design canvas, off by default" (2026-07-11)
- [live-mcp-v2.md](../live-mcp-v2.md), [agent quickstart](../fanta/agent-quickstart.md), #155
