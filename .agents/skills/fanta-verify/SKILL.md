---
name: fanta-verify
description: Choose and run the right checks for a Fanta change - crate tests, script/verify-fanta-release stages, CI-only gates, fig_viewer probe and render examples, the live MCP smoke test, and native QA caveats. Use before claiming a change works, or when asked to test, verify or validate a change.
---

# Verifying a Fanta change

Run the narrowest check that exercises the change, then the release stage that covers its area. Report what you ran and what you did not.

`script/gate` runs the checks that need no compile, in seconds. `script/gate --full` then runs `./script/clippy -p` for the crates the branch changed since `origin/main`, the release stages that name them, and `cargo test -p` for changed crates no stage covers; `--list` prints the commands. It does not choose the single-test gates, probes, smoke tests or platform checks in the table below.

## Pick the checks

| Changed | Run |
| --- | --- |
| `fanta-doc`, `fanta-canvas`, `fanta-tools`, `fanta-text`, `fanta-present` | `cargo test --locked -p <crate>`, then stage `engine` |
| `fanta-render` | `cargo test --locked -p fanta-render`, then stage `render`. Its golden image was rendered on macOS, so `tier1_golden_scene_matches` fails on Linux; do not regenerate it |
| `fanta-fnx`, `fanta-format` | `cargo test --locked -p fanta-fnx -p fanta-format --lib`, then stages `engine` and `source-order` |
| `fanta-fig-interop` | stage `import`; with a private fixture, `FANTA_FIG_FIXTURE=<file.fig>` and stage `import-preservation` |
| `fig_viewer` | `cargo test --locked -p fig_viewer --lib <filter>`, then stage `editor`; add `workspace` for pane, save and close flows |
| `crates/workspace` | stage `workspace` |
| `fanta_ui` | stage `ui`; stage `visual` compares UI images and needs a macOS desktop |
| Built-in agent canvas tools or prompts | `cargo test --locked -p agent --lib test_fanta_specs` and `test_design_progress` (both CI gates) |
| Live MCP server or agent surface | the agent tests above, plus `script/smoke-mcp` against a running app (below) |
| Anything compiled into the app | stage `build` (`cargo check --locked -p fanta -p cli`) |
| Lints | `./script/clippy -p <crate>` (off macOS it enables every feature except the renderer's Metal ones); stage `lint` covers the release crates and lints the wasm engine for the browser target |
| Platform-specific code (`cfg(target_os)`, `std::os::unix`, per-OS keymaps) | the `Windows` workflow (`.github/workflows/windows.yml`). On Windows, run `.\script\clippy.ps1` and the `cargo` commands directly; the other scripts are Bash ([guide](../../../docs/fanta/windows-development.md)) |
| Packaging scripts | `bash -n <script>`, `python3 script/test-macos-release`, `./script/check-licenses` |
| Crate dependencies, a new crate, `docs/architecture`, `docs/decisions`, `specs`, `script/architecture` | `script/architecture verify` (no build); `python3 script/test-architecture` when the script changed |
| `mac_app_store` code paths | `cargo check --locked -p fanta --features mac_app_store`, plus the matching tests in the `mac-app-store-runtime` job of `check.yml` |

`./script/verify-fanta-release --list` prints each stage's exact command. `--stage <name>` (repeatable) runs stages and writes logs plus `results.tsv` under `target/release-verification`. The script does not validate an installer, drive the full app, contact production services or run ignored benchmarks, and it refuses to run while golden-regeneration variables are set.

Stage `shell` runs the single-test gates on the inherited crates (bundled git, languages, AI provider auth, browser sign-in, Save As prompts, update verification, agent panel toggle, sidebar) and stage `release-blockers` the agent request-limit, output-limit, compaction and Settings regressions with their default-feature lint. Off macOS both skip, with a printed note, the steps whose tests are macOS-only (bundled git, the Settings window).

A green stage is not the whole CI gate. `.github/workflows/check.yml` also requires 51 named editor and workspace tests. Renaming one of those tests means updating that list.

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

## Fuzzing the parsers

The four crates that read files from outside each have a `fuzz/` directory (`cargo fuzz` layout, its own workspace and `Cargo.lock`, outside the root workspace): `fanta-fig-interop` (`read_fig`, `kiwi`), `fanta-fnx` (`parse`), `fanta-format` (`read_project`) and `fanta-bundle` (`load_bundle`). `.github/workflows/fuzz.yml` runs each for five minutes.

- Run one with `script/fuzz <crate> <target> [seconds]`, for example `script/fuzz fanta-fnx parse 60`. It needs `cargo install --locked cargo-fuzz@0.13.2` and the nightly that `script/fuzz --toolchain` prints. The script sets the input size, time and memory limits; do not put limits in a target.
- Committed seeds are in `fuzz/seeds/<target>`. The `read_project` seeds are generated from `crates/fanta-format/tests/corpus`: run `script/fuzz-seeds` after that corpus changes. Inputs the run finds go to the ignored `fuzz/corpus`, and a crash to `fuzz/artifacts/<target>/`.
- To turn a crash into a regression test: shrink it with `cargo +<nightly> fuzz tmin <target> <artifact>` from the crate directory, fix the parser so it returns the crate's error, add a unit test in the crate that feeds the shrunk bytes, and copy the input into `fuzz/seeds/<target>`. Do not commit or post the input before the fix is merged.
- The repository is public, so the nightly job neither uploads nor prints a crashing input: it shows the target, libFuzzer's seed and the input's SHA-256. To reproduce, run the target locally for the same five minutes on the same commit, with `-seed=<n>` added to the libFuzzer arguments in `script/fuzz`, and compare the SHA-256 of the file in `fuzz/artifacts/<target>/`.
- Each `fuzz/Cargo.lock` is the root `Cargo.lock` cut down to what the target needs, so the fuzzer runs the dependency versions the app ships. After the root lock changes a parser's dependencies, copy it over `fuzz/Cargo.lock` and run `cargo metadata` in `fuzz/` to trim it. `script/fuzz` checks the lock with `cargo metadata --locked` first (cargo-fuzz has no `--locked`), so a stale one fails the run.
