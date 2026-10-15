---
name: fanta-perf
description: Measure and investigate Fanta canvas and document performance - the navigation and drag benchmarks, FANTA_PERF tracing, macOS memory measurement, and the traps that make numbers misleading. Use for canvas lag, frame rate, load time or memory work on large documents.
---

# Fanta performance work

## Before measuring

- Copy the project you measure. Never point a benchmark at the user's working project.
- Keep other builds off the machine during timed runs. Build in your checkout's default `target/` (do not set `CARGO_TARGET_DIR`) instead of killing the user's editor or rust-analyzer processes.
- Dev builds keep Skia fast only because of `[profile.dev.package] skia-bindings = { opt-level = 3 }` in `Cargo.toml`; without it, a 12 MP JPEG decode takes 1.2 s instead of 51 ms. Keep that override.

## Navigation benchmark

`crates/fig_viewer/src/canvas/navigation_bench.rs` drives the production canvas paint planner and Metal render thread with scripted camera input at 60 Hz:

```sh
FANTA_PERF=1 FANTA_CAMERA_BENCH_PROJECT=<project copy> \
FANTA_CAMERA_BENCH_OUT=<result.json> \
cargo test -p fig_viewer --lib navigation_bench -- --ignored --nocapture
```

The project must contain a `.claude-benchmark-fixture` marker file; the benchmark refuses to run without it. Other variables: `FANTA_CAMERA_BENCH_SCENARIOS`, `_SNAPSHOTS`, `_REFERENCE`, `_FOOTPRINT`, `_PAGE`, `_OTHER_PAGE`, `_SIZE` (read the module for their meaning).

## Drag benchmark

`cargo run -p fig_viewer --example drag_bench -- <file.fig> [options]` moves one node and reports timing as JSON. It measures CPU raster rendering by default, and `--metal-retained` compares full Metal rendering with the retained GPU path. Neither measures GPUI input latency or displayed frame rate.

## Measurement traps

- A starved build's "settled" frame can be the frame from before the motion. Compare against direct references (`FANTA_CAMERA_BENCH_REFERENCE=1`), not two builds' snapshots.
- First-frame timings are bimodal around vsync (about 16.7 ms apart). Repeat runs before calling a shift a regression.
- `render_wall_us` excludes buffer allocation and texture import.
- Tile composites are not bit-exact with direct renders on real content: a few antialiased pixels differ, and small text rows shift by 1 px at 10% zoom. This predates current work.
- mimalloc tags its heap with VM tag 100, which `vmmap` and `footprint` label "IOAccelerator"; that is the Rust heap. GPU memory is "IOAccelerator (graphics)" and "Owned physical footprint (unmapped) (graphics)".
- Command-line processes (benchmarks, scripts) carry a 200–350 MiB graphics ledger that the GUI app does not, and `phys_footprint` swings with malloc retention. Compare heap in use, Metal allocation and cache sizes instead.
- The canvas renders only while its window is visible; GPUI stops the display link when the window is occluded.

## Other tools

- `cargo perf-test` and `cargo perf-compare` (aliases in `.cargo/config.toml`) need `cargo install fanta-gpui-perf --version 0.1.0 --locked`.
- `docs/performance/` records earlier investigations.
