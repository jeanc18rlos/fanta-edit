# fanta-headless rules

The editor's document code that runs without the app: opening a project, settling layout, the comment model, the agent scene and style projections, screenshots and the source-preserving save. `fig_viewer` re-exports it, and the cloud engine (`crates/fanta-worker`) runs it on Linux.

PR numbers point at the change that found each trap (`gh pr view <n>`).

- Never depend on `gpui`, `fanta_ui` or another UI crate, directly or through a dependency: the engine builds without them. `cargo tree -p fanta-headless -e normal | grep -i gpui` must print nothing. Helpers that need a UI crate stay in `fig_viewer` next to the re-export, as the comment-skill prompts and timeline labels in `fig_viewer/src/comments.rs` do. (#157)
- Change shared behavior here, never in a copy inside `fig_viewer`: the editor and the engine must open, settle, render and save a project the same way. `fig_viewer`'s `headless_engine_renders_match_editor_screenshots` compares their renders byte for byte. (#157)
- The browser engine (`crates/fanta-wasm-engine`) builds this crate without `native` and `render`, a configuration no native build or lint compiles. Keep helpers that non-rendering code calls out of `render`, and gate code that names native-only `fanta-format` items (the session, `SessionError`) on `native`. Check with `RUSTFLAGS='--cfg getrandom_backend="wasm_js"' cargo check --locked -p fanta-wasm-engine --target wasm32-unknown-unknown`. (#176)
- `HeadlessProject::save` records snapshots in the project's `.fanta/history.git` unless its host called `set_snapshots(false)`. A host that saves a copy of someone else's project (the cloud engine) must turn them off, and must keep `.fanta` and `.git` out of that copy, so that it never starts git on files a caller supplied. (#240)
