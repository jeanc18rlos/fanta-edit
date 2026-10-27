---
status: superseded
date: 2026-07-07
recorded: 2026-10-27
decision-makers: Jean Rojas
superseded-by: 0007
tags: [agents]
---

# Let agents edit designs through project files

## Context

Zed's agent panel came with the fork and was more capable than the first
app's AI integration. An agent tool that touched the live document would have
made the `agent` crate depend on the editor, which is the wrong direction for
a crate inherited from Zed.

## Decision

Agents change designs by editing the project's `.fnx` and JSON files as text.
The canvas reloads from disk when it has no unsaved edits. Every new project
gets an `AGENTS.md` design-source guide, seeded by `fanta-format`. No live
selection or viewport file is written, to avoid races with the file watcher.

## Options considered

1. **Files only** (chosen at the time). No new dependency, and it matches the
   design-is-source thesis.
2. **A native canvas tool in the agent crate.** Rejected then because the
   agent crate would depend on the editor.

## Consequences

- Good: any agent that edits files could change a design.
- Bad: agents could not see the canvas, the selection or a rendered result,
  and every edit cost a save and reload cycle.
- Superseded on 2026-07-11 by a seam crate that solved the dependency problem
  ([0007](0007-give-agents-a-live-design-seam-and-mcp-server.md)). Editing
  project files remains a supported path.

## Confirmation

None today; see 0007.

## References

- Founder decision recorded in session notes on 2026-07-07
- `AGENTS_MD` seed in `crates/fanta-format/src/project/layout.rs`
