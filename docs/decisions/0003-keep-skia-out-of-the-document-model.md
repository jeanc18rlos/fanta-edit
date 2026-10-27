---
status: accepted
date: 2026-07-09
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [engine, rendering]
---

# Keep Skia out of the document model

## Context

Fanta renders with Skia, and its effects pipeline and text shaping lean on
Skia features. The document model, the interaction math and the file formats
have no need for a renderer. If they linked Skia, every consumer of the model
(the format crate, the importers, a future collaboration server or SDK) would
build Skia too, and a second renderer would mean rewriting the model.

## Decision

The renderer seam is a dependency direction, not a trait. The model, the
design-op vocabulary, interaction, and the source and import crates never
depend on Skia, directly or through another crate. Only the rendering crates
(`fanta-text`, `fanta-render`) and the crates above them may. Hosts give the
renderer decoded images through `AssetResolver`, so `fanta-render` never
depends on `fanta-format`. Do not add an abstract renderer trait until a
second backend is planned: an interface designed against one implementation
would either copy Skia's API or lose fidelity.

## Options considered

1. **Dependency direction, checked mechanically** (chosen).
2. **A `Renderer` trait now.** Costs an abstraction with one implementation
   and no second backend to shape it.
3. **No rule.** The model would pick up Skia types the first time it was
   convenient.

## Consequences

- Good: `fanta-doc`, `fanta-fnx`, `fanta-format` and `fanta-fig-interop`
  build without Skia, which keeps their tests fast and lets the runtime
  compiler and the format tools stay small.
- Bad: a second renderer still means real work in the rendering crates. The
  seam keeps that work out of the model; it does not remove it.

## Confirmation

The layer map forbids Skia in the Model, Design-op vocabulary, Interaction,
and Source and import layers, and `script/architecture check` fails when one
of their crates reaches `skia-safe` or `skia-bindings`.

## References

- The engine repository's `ARCHITECTURE.md` §2, added in commit `3f65e68` (2026-07-09) with the original `cargo tree` tripwire. That commit is on the never-pushed local branch described in [0008](0008-vendor-the-engine-into-this-repository.md).
- [`layers.toml`](../architecture/layers.toml)
