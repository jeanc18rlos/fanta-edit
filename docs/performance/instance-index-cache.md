# Cache instance child indexes

On 2026-10-05, retaining each instance expansion’s child index reduced median run CPU render p95 at fit-all from **150.037 to 146.675 ms** for an instance and **148.952 to 147.521 ms** for an image-filled frame. These are small observed reductions of 2.24% and 0.96% across three runs. They do not establish native gesture latency, release-build performance, or the below-16-ms target.

The renderer previously rebuilt the parent-to-child slot map and searched for the root after every expansion-cache hit. `PreparedInstance` now retains both alongside the immutable expanded nodes. Existing keys, master/preview revisions, override/mode invalidation and eviction remain unchanged; sibling order is preserved. Cached indexes add retained memory proportional to the cached expansion edges. **RSS was not measured**, so no memory improvement is claimed.

## Matched protocol

Apple M3 Max, 36 GiB RAM; Rust `1.95.0 (59807616e 2026-04-14)`; Cargo dev profile with workspace package optimization overrides. Each invocation imports the unchanged Spectrum file, uses a 1280×800 CPU raster surface, warms three frames, and measures 120 transform/render frames at fit-all and 100%. Three repetitions ran sequentially, instance then image, with the native QA app idle and no concurrent compilation. Import/layout/decode time is outside the frame measurements.

Both sides selected page index 9, “↳  🌙  Darkest Theme”, with 8,636 scene nodes. Node index 4358 remained `ChevronDown` (instance), bounds `[7712, 2265, 7732, 2285]`; index 2110 remained `Style=Image, State=Default` (frame), bounds `[17548, 1105, 17588, 1145]`. Fit-all zoom was `0.022514599310490397`. Names, kinds, indices and bounds matched in every run.

```sh
cargo build --locked -p fig_viewer --example drag_bench
# Repeat this pair three times after compilation finishes.
target/debug/examples/drag_bench "/path/to/Adobe Spectrum Design System .fig" --page-index 9 --node 4358 --frames 120
target/debug/examples/drag_bench "/path/to/Adobe Spectrum Design System .fig" --page-index 9 --node 2110 --frames 120
```

## Results

All times below are milliseconds. Each cell gives **p50 / p95 / maximum** for one 120-frame run.

| Target / zoom | Run | Before | After |
| --- | ---: | ---: | ---: |
| Instance, fit-all | 1 | 144.494 / 148.012 / 148.890 | 142.425 / 145.714 / 152.302 |
| Instance, fit-all | 2 | 145.699 / 151.262 / 158.724 | 143.359 / 146.675 / 156.573 |
| Instance, fit-all | 3 | 145.651 / 150.037 / 168.309 | 142.897 / 148.399 / 158.012 |
| Instance, 100% | 1 | 8.024 / 8.321 / 8.735 | 7.988 / 8.302 / 8.979 |
| Instance, 100% | 2 | 8.173 / 8.505 / 8.705 | 8.080 / 8.458 / 9.305 |
| Instance, 100% | 3 | 8.172 / 8.490 / 9.043 | 8.024 / 8.271 / 8.758 |
| Image frame, fit-all | 1 | 145.122 / 149.468 / 159.578 | 143.325 / 147.235 / 161.417 |
| Image frame, fit-all | 2 | 145.050 / 148.952 / 154.097 | 143.258 / 147.532 / 156.600 |
| Image frame, fit-all | 3 | 144.972 / 148.825 / 153.089 | 143.195 / 147.521 / 151.957 |
| Image frame, 100% | 1 | 1.203 / 1.376 / 1.479 | 1.209 / 1.410 / 1.530 |
| Image frame, 100% | 2 | 1.195 / 1.468 / 1.506 | 1.207 / 1.397 / 1.733 |
| Image frame, 100% | 3 | 1.211 / 1.367 / 1.602 | 1.190 / 1.354 / 1.457 |

At 100%, median run p95 changed from 8.490 to 8.302 ms for the instance, and from 1.376 to 1.397 ms for the image frame (0.021 ms slower). Maximum frame times did not improve in every condition. Three sequential repetitions support a bounded allocation reduction; they are not a statistical latency guarantee.

Across all runs, visited/drawn/culled node counts, existing path/effect/cache counts and error flags matched between versions. Incomplete-artwork, non-artwork-content and effect-failure counts were zero. The new `instance_indexes_built` counter was **zero across every measured frame** after warmup, at both zooms for both targets. Its baseline value is unavailable because the counter is new. Existing `paths_built` excludes uncached transient vector builds and must not be interpreted as complete path-cache coverage.

## Provenance and retained evidence

Baseline build revision: `ee80ec48c642b42b704d674da2a87dc8c7f42a67`. The documentation-only commit `e30a0a2715e115e892dde5111d1e49b5c45c580c` landed during the baseline runs; it changed neither measured source nor binary. Optimized measurements used that base with this four-file performance patch. The existing unrelated pointer UI change remained unchanged and is outside this patch.

| Artifact | SHA-256 |
| --- | --- |
| Spectrum source, 21,526,600 bytes | `8130233d07c31d41bfb980ecd37aac91563d1c3377d6e32eee772910157ba3ab` |
| Before benchmark binary | `ee8fb0bd01aa466e8fbbfc8132525adf99bc2a1223ecd08ceebdc24d30a6f3a1` |
| After benchmark binary | `6e09ce34b2ed54f5f80e8baf892baec3621fcfee504cc5a1f5860cd18a97a47a` |
| Prepared four-file patch | `eb9c1487fc39cae3c4720c11b80ecb70b1c7f4256d42175e4b84705218e02db2` |

The source file and each measured binary remained byte-identical throughout their runs. The optimized source hashes also remained unchanged throughout timing. The baseline’s later source check occurred after the timing lock was released and the patch applied; metadata records this ordering explicitly. Minimum free disk was 32.478 GiB. No input fixture was modified.

The following links are **local, ignored evidence artifacts**, not files shipped in the repository. They retain every JSON result, stderr log, command, timestamp and per-file source hash:

- [Matched comparison and all run statistics](../../target/release-verification/instance-index-cache-20261005/matched-comparison.json)

- [Before fingerprints and run manifest](../../target/release-verification/instance-index-cache-20261005/baseline-ee80ec48/metadata.json)

- [After fingerprints and run manifest](../../target/release-verification/instance-index-cache-20261005/after-e30a0a2/timing/metadata.json)

- [Reusable measurement runner](../../target/release-verification/instance-index-cache-20261005/run_matched_benchmark.py)

## Validation

- `cargo test --locked -p fanta-render --lib`: **341 passed, 0 failed, 2 ignored**. [Full log](../../target/release-verification/instance-index-cache-20261005/after-e30a0a2/renderer-tests.log).

- `cargo test --locked -p fig_viewer --example drag_bench`: **3 passed**. [Full log](../../target/release-verification/instance-index-cache-20261005/after-e30a0a2/benchmark-cli-tests.log).

- `./script/clippy --locked -p fanta-render -p fig_viewer`: **passed**, including the script’s dependency check. [Full log](../../target/release-verification/instance-index-cache-20261005/after-e30a0a2/clippy.log).

- `cargo build --locked -p fig_viewer --example drag_bench`: **passed**. [Full log](../../target/release-verification/instance-index-cache-20261005/after-e30a0a2/benchmark-build.log).

New regressions verify zero index rebuilds during transform frames, exact pixels against a fresh renderer, rebuilds after master reordering and Undo, preview cancellation, and equal-index child order/root lookup. The complete renderer suite also covers instance overrides, aliases/modes, masks and Boolean instance rendering. [Validation command manifest](../../target/release-verification/instance-index-cache-20261005/after-e30a0a2/validation.json).

Release note: Improved canvas rendering by reusing cached component-instance child indexes during redraws.
