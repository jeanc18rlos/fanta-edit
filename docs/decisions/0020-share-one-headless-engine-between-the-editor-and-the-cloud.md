---
status: accepted
date: 2026-10-21
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [engine, architecture]
---

# Share one headless engine between the editor and the cloud

## Context

Cloud review and generation must open a project, apply edits, settle layout,
render pages and save, without the app. That code lived in `fig_viewer`,
mixed with GPUI. A second copy for the cloud would drift from the editor's
behavior within weeks.

## Decision

`fanta-headless` is the facade both sides share. The editor's project
loading, layout settling, comment model, scene and style projections,
screenshot rendering, page operations and source-preserving save move into it
unchanged (#157), and `fig_viewer` calls and re-exports the same code. The
design-op applier, image ingestion and the GPUI-free edit helpers followed on
2026-10-25 (#160). Rules for the crate:

- never depend on `gpui`, `fanta_ui` or another UI crate, directly or
  through a dependency;
- change shared behavior here, never in a copy inside `fig_viewer`.

## Options considered

1. **Move the code into one shared crate** (chosen).
2. **Run the app headless in the cloud.** The app needs a window system and
   the macOS stack; the cloud worker runs on Linux.
3. **Reimplement for the cloud.** Two behaviors to keep equal by hand.

## Consequences

- Good: the editor and the cloud open, settle, render and save a project the
  same way, and this is the first step toward an editor that is a thin host.
- Bad: much application logic is still in `fig_viewer`, such as the file
  watcher, reconciliation and three-way merge in `document.rs`; see
  [0023](0023-enforce-layered-crate-boundaries.md).

## Confirmation

`fig_viewer`'s `headless_engine_renders_match_editor_screenshots` compares
the editor's and the engine's renders byte for byte. The layer map forbids
`fanta-ui` anywhere in the engine group.

## References

- #155 and #157 (2026-10-21), #160 (2026-10-25)
- [crates/fanta-headless/AGENTS.md](../../crates/fanta-headless/AGENTS.md)
