# Fanta Edit BDD Feature Specifications

These are human-readable Gherkin (`.feature`) files describing the expected behavior of Fanta Edit's visual design capabilities.

They serve two purposes:
1. Living documentation of features (easy for humans, PMs, or future contributors to read).
2. Basis for E2E / acceptance tests.

## How to use

- Read the `.feature` files to understand product behavior.
- Map scenarios to automated tests (currently implemented as `#[gpui::test]` in `crates/fig_viewer/src/view.rs`, `agent_surface.rs`, etc.).
- When adding new features (image editor, 3D, etc.), add corresponding `.feature` files first.

## Running / Implementing

The tests use GPUI's test harness (`gpui::test` + `TestAppContext`).

Example existing coverage:
- Canvas interactions, undo, motion, prototypes, viewport persistence, FNX locking, design ops batch semantics.

See `crates/fig_viewer/src/view.rs:mod tests` and `agent_surface.rs` for current E2E-style tests.

## Future

- Full Cucumber-rs or custom runner can consume these `.feature` files.
- Visual regression uses the snapshot images under `target/visual_tests/`.

## Key Features Documented

- Visual canvas authoring (shapes, frames, text, selection, transforms)
- FNX <-> visual bidirectional sync with locking
- Motion, timeline, prototypes
- Agent / DesignSurface tools (design_state, design_edit, design_screenshot)
- Code + visual workspace integration
- Panels, variables, comments

Add more `.feature` files for audio, video, 3D, game editors as they are developed.
