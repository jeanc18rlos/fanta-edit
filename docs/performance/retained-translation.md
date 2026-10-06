# Retained translation rendering

The renderer can prepare an opt-in translation session for one eligible node. This is not enabled in the canvas worker. The first matched Spectrum CPU experiment passed all 12 runs and 552 full-frame pixel comparisons with zero differing channels. It shows a useful full-page optimization and a clear reason not to enable this path for every drag.

## Matched Spectrum results

The test reads the unchanged Spectrum project, moves Image2110 or Instance4358 on the Darkest Theme page, and compares normal rendering with the retained session at the same viewport and transform. Each target/zoom has three rounds of 120 measured pairs, with the first renderer alternating each frame. All 40 timed positions plus six controls are checked for pixel parity before that run's measurements. Readback and parity are outside the timers; per-frame semantic validation is inside them.

The table reports the median of the three round quantiles, in milliseconds. Full-page zoom is approximately 2.25146%; output is 1280×800 physical pixels. The executable uses the recorded development library build, not an optimized application release.

| Target | View | Normal p50 | Retained p50 | Normal p95 | Retained p95 | Median session setup |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Image2110 | Full page | 151.620 | 16.799 | 158.233 | 18.077 | 255.591 |
| Instance4358 | Full page | 152.099 | 9.538 | 159.464 | 10.439 | 245.553 |
| Image2110 | 100% | 1.469 | 9.170 | 2.268 | 10.546 | 119.117 |
| Instance4358 | 100% | 9.999 | 10.471 | 11.182 | 11.388 | 116.542 |

At full-page zoom the retained path substantially reduces CPU time. At 100%, Image2110 becomes slower and Instance4358 shows no improvement. Session setup costs 114.743–281.855 ms across these runs and must remain off the UI thread. A future live path needs a measured cost and amortization gate; zoom alone is not sufficient. These results do not establish native input latency, GPU performance, release-build performance, or a 16 ms frame guarantee.

The raw paired samples retain all outliers, including one 1,577.103 ms normal-render sample for the full-page instance. The summary also retains pooled quantiles and each individual round; the table does not discard samples.

## What is retained

A session prepares exact instance expansions and frozen asset inputs, paints Below once, and records Above as ordered Skia commands. Each accepted translation copies Below, draws the moving Middle, and replays Above. Ordered replay preserves backdrop-dependent blends such as Screen and closed mask runs. Independently compositing a rasterized Above is refused when it cannot preserve the paint order.

Painted ancestors may allow overflowing children when their background, border and effect silhouette use an explicit positive `clip_size` or `local_size`. This matches the existing painter: the fixed paint box and child clipping are separate decisions. Unbounded ancestor paint that derives from descendant bounds remains ineligible. A regression matrix covers both extent kinds, reverse order, reflections, translations, zoom/DPI, visible overflow, foreground borders, and refusal without modifying the target.

Median per-frame costs pooled across the three rounds are:

| Target/view | Exact validation | Below copy | Middle | Above replay |
| --- | ---: | ---: | ---: | ---: |
| Image, full page | 3.149 | 0.078 | 5.321 | 8.046 |
| Instance, full page | 3.130 | 0.078 | 0.221 | 5.980 |
| Image, 100% | 3.118 | 0.077 | 5.465 | 0.355 |
| Instance, 100% | 3.155 | 0.078 | 0.228 | 6.912 |

These independently computed medians need not sum to the median total. Exact component, variable, mode and asset checks still scan their captured inputs each frame. Middle's visited-node counter does not include all skipped static traversal, mask-bound work or Above command replay. No full document or registry clone occurs inside the measured retained frame call; preparation owns the required registry snapshots.

## Correctness and resource boundary

The session accepts only a complete transform-only scene delta for the same single root, with finite translation and unchanged linear transform. Scene identity, page, camera, output size, scale, background, font generation, bindings, component metadata, variables, modes and asset pixels must remain consistent. Preview revisions may advance only for proved unaffected containing definitions; any prepared instance consuming those definitions makes the session ineligible. The two recorded Spectrum targets have no such consumers, but production checks the actual prepared context rather than relying on those fixture IDs.

Any failed proof permanently disables that session. It does not replace the last valid output. Structural edits, committed component changes, untracked or expired deltas, dynamic video/motion, unsupported paints, unsafe masks, and backdrop effects retain explicit refusal paths. The caller must provide a font generation that changes with the font environment.

For this fixture the account includes 199,164,236 bytes of frozen decoded pixels, 8,192,000 bytes for two pixel surfaces, and approximately 9,504–1,644,664 bytes of Picture storage depending on the viewport/target. This is below the current 256 MiB account limit. It is not a total-memory or peak-allocation limit: process memory, Skia internals, owned registries, prepared nodes and decoding temporaries are not fully represented by that account.

## Validation and next integration gate

The fixed-paint regression first reproduced two expected eligibility failures while the invalid-extent control passed. After the narrow change, all three new tests passed, the full renderer suite passed 389 tests with two ignored, and renderer/viewer lint passed. The real-project experiment then passed all 12 eligibility/parity/timing gates: 552 parity frames, 1,440 measured pairs, and zero retained instance-index rebuilds. All 27,162 project files and symlink targets remained exact, including 166,757,318 bytes of file content. The authored in-memory document was also restored exactly.

The next step is an opt-in worker protocol and actual-backend probe, not default activation. It must preserve asynchronous dispatch, gesture tokens, cancellation and stale-reply rejection, keep normal-render cost history separate, and bound session lifetime/resources. The current session owns CPU raster surfaces; composing its output into the Metal canvas introduces an additional backend/upload cost that this experiment does not measure. A backend-equivalent comparison must include that cost before any retained frame is presented.

Evidence is under `target/release-verification/renderer-fixed-paint-ancestors-20261005/integration/`: `final-validation.json`, `harness-link.json`, `run-context.json`, `spectrum-cpu-run1/report.json`, `spectrum-summary.json`, and `project-unchanged.json`. These ignored local artifacts record exact source, executable, library, log and project hashes; they are not bundled into the release. Earlier setup failures and conservative refusals remain preserved in the preceding retained-session evidence.
