---
status: accepted
date: 2026-10-14
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [rendering, dependencies]
---

# Render the Metal canvas with Skia Graphite

## Context

The Metal canvas was tied to Skia's Ganesh context and snapshot APIs on
`skia-safe` 0.84, and large documents such as Adobe Spectrum's stalled while
navigating. Graphite is Skia's newer GPU backend, but the published
`skia-bindings` 0.153.3 could not drive it correctly: Skia m153's default
recorder drops raster-backed image and shader draws, and the generator broke
on Apple clang 21 with the macOS 27 headers.

## Decision

Upgrade to `skia-safe` 0.153.3 and make Graphite the default Metal backend,
keeping CPU exports and a Ganesh fallback for start-up and diagnostics. Vendor
the published `skia-bindings` archive under `vendor/skia-bindings-0.153.3`
with three changes, each documented in its `FANTA-PROVENANCE.txt`: the
upstream generator fix from rust-skia PR 1335, a bounded image provider per
recorder, and native exports for a worker-thread pipeline executor and GPU
memory controls. Graphite builds compile Skia from source.

## Options considered

1. **Graphite through a vendored, patched binding** (chosen).
2. **Stay on Ganesh.** No path to the retained and progressive rendering that
   large documents needed.
3. **Wait for upstream.** The image-provider gap had no upstream fix.

## Consequences

- Good: with the camera and tiling work in #129, navigating large documents
  stays responsive, and pipeline compilation runs on worker threads.
- Bad: a fresh checkout compiles Skia from source, about twenty minutes, and
  needs Xcode's Metal toolchain. Every Skia upgrade means re-applying the
  shim. #136 added a content-keyed local cache to soften the rebuilds.

## Confirmation

`FANTA-PROVENANCE.txt` lists every changed file against the published
archive's checksum. The `skia-source-compatibility` workflow builds the
vendored binding, and the renderer's parity tests compare Graphite output
with CPU output.

## References

- #127 (closed; its commit `13e5eb2742` landed through #129 on 2026-10-14), #134, #136
- [FANTA-PROVENANCE.txt](../../vendor/skia-bindings-0.153.3/FANTA-PROVENANCE.txt)
