# Fanta Edit BDD Feature Specifications

These are human-readable Gherkin (`.feature`) files describing the expected behavior of Fanta Edit's visual design capabilities.

They are acceptance specifications, not an executable test suite. No runner
currently consumes the `.feature` files. The current capability inventory and
specific test mappings are in [the capability guide](../docs/fanta/capabilities.md)
and [release coverage report](../docs/alpha/CAPABILITY_COVERAGE.md).

## How to use

- Read the `.feature` files to understand product behavior.
- Map scenarios to engine tests and `#[gpui::test]` cases in
  `crates/fig_viewer/src/view.rs`, `agent_surface.rs`, and the other feature modules.
- When adding new features (image editor, 3D, etc.), add corresponding `.feature` files first.

## Running / Implementing

Run `./script/verify-fanta-release` for the public engine, GPUI integration and
build checks. Use `--list` to inspect commands or `--stage editor` for the editor
suite. A passing harness test does not establish native app E2E coverage.

Example existing coverage:
- Canvas interactions, undo, motion, prototypes, viewport persistence, FNX locking, design ops batch semantics.

See `crates/fig_viewer/src/view.rs:mod tests` and `agent_surface.rs` for integration
tests. Some inject pointer/key input; others call operations directly.

## Future

- Full Cucumber-rs or custom runner can consume these `.feature` files.
- Native component visual regression is available with
  `./script/verify-fanta-release --stage visual` on macOS. It compares component
  galleries to committed images in `crates/fanta_ui/test_fixtures/visual/macos/`;
  generated results appear under `target/visual_tests/`. It does not snapshot the
  complete assembled editor or every inspector state.

## Key Features Documented

- Visual canvas authoring (shapes, frames, text, selection, transforms)
- FNX <-> visual bidirectional sync with locking
- Motion, timeline, prototypes
- Agent / DesignSurface tools (design_state, design_edit, design_screenshot)
- Code + visual workspace integration
- Panels, variables, comments

Add more `.feature` files for audio, video, 3D, game editors as they are developed.
