---
name: fanta-verify
description: Choose and run the right checks for a Fanta change - crate tests, script/verify-fanta-release stages, CI-only gates, fig_viewer probe and render examples, the live MCP smoke test, and native QA caveats. Use before claiming a change works, or when asked to test, verify or validate a change.
---

# Verifying a Fanta change

Run the narrowest check that exercises the change, then the release stage that covers its area. Report what you ran and what you did not.

## Pick the checks

| Changed | Run |
| --- | --- |
| `fanta-doc`, `fanta-canvas`, `fanta-tools`, `fanta-render`, `fanta-text`, `fanta-present` | `cargo test --locked -p <crate>`, then stage `engine` |
| `fanta-fnx`, `fanta-format` | `cargo test --locked -p fanta-fnx -p fanta-format --lib`, then stages `engine` and `source-order` |
| `fanta-fig-interop` | stage `import`; with a private fixture, `FANTA_FIG_FIXTURE=<file.fig>` and stage `import-preservation` |
| `fig_viewer` | `cargo test --locked -p fig_viewer --lib <filter>`, then stage `editor`; add `workspace` for pane, save and close flows |
| `crates/workspace` | stage `workspace` |
| `fanta_ui` | stage `ui`; stage `visual` compares UI images and needs a macOS desktop |
| Built-in agent canvas tools or prompts | `cargo test --locked -p agent --lib test_fanta_specs` and `test_design_progress` (both CI gates) |
| Live MCP server or agent surface | the agent tests above, plus `script/smoke-mcp` against a running app (below) |
| Anything compiled into the app | stage `build` (`cargo check --locked -p fanta -p cli`) |
| Lints | `./script/clippy -p <crate>`; stage `lint` covers the release crates |
| Platform-specific code (`cfg(target_os)`, `std::os::unix`, per-OS keymaps) | the `Windows` workflow (`.github/workflows/windows.yml`). On Windows, run `.\script\clippy.ps1` and the `cargo` commands directly; the other scripts are Bash ([guide](../../../docs/fanta/windows-development.md)) |
| Packaging scripts | `bash -n <script>`, `python3 script/test-macos-release`, `./script/check-licenses` |
| `mac_app_store` code paths | `cargo check --locked -p fanta --features mac_app_store`, plus the matching tests in the `mac-app-store-runtime` job of `check.yml` |

`./script/verify-fanta-release --list` prints each stage's exact command. `--stage <name>` (repeatable) runs stages and writes logs plus `results.tsv` under `target/release-verification`. The script does not validate an installer, drive the full app, contact production services or run ignored benchmarks, and it refuses to run while golden-regeneration variables are set.

A green stage is not the whole CI gate. `.github/workflows/check.yml` also runs single-test gates (bundled git, languages, AI provider auth, browser sign-in, Save As prompts, agent panel toggle, sidebar) and requires 51 named editor and workspace tests. Renaming one of those tests means updating that list.

## Probe and render examples

Run `cargo run -p fig_viewer --example <name> -- <arguments>`; each example's header comment documents its arguments.

- `render_fig_page`: render one page of a `.fig` to PNG. Compare it with the file's embedded thumbnail (`unzip -o -j <file.fig> thumbnail.png`).
- `inspect_fig_page`, `inspect_fig_tree`, `dump_raw_fig_nodes`: dump node trees and raw fields.
- `probe_fidelity`, `probe_group_state`, `probe_instance_link`, `probe_instance_text`: targeted import and instance diagnostics.
- `fig_to_project`, `make_instance_project`: build test projects.
- `export_fig_frames`, `drag_bench`, `fanta_gpui_smoke`: frame export, drag timing, platform smoke check.

## End-to-end agent loop

`script/smoke-mcp <fanta-binary> <project-dir>` starts or attaches to the app, edits the canvas over MCP, and checks that the edit reaches the project's `.fnx` source without a manual save. Options: `--launch-timeout`, `--autosave-timeout`, `--quit-app`, `--socket`, `--screenshot-path`. Set `FANTA_USER_DATA_DIR` to use an isolated profile. Run it against a copy of a project, never the user's working one.

## Native QA with computer use

- Synthetic keystrokes do not reach GPUI inspector fields and do not deliver Escape. Check typed input with mounted GPUI tests or a real keyboard.
- Start numeric scrub checks on the visible numeric readout; the leading icon is outside the scrub surface.
- If the user asked for code-only work, skip app launches and say so.

## Report

List the commands you ran with their results, the stages you skipped and why, and anything you verified only by reading code.
