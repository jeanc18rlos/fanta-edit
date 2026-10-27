---
status: proposed
date: 2026-10-27
decision-makers: Jean Rojas
tags: [architecture, engine]
---

# Enforce layered crate boundaries

## Context

Fanta's code is split into crates, but the rules between them were implicit.
Two rules existed only as one-off tripwires: `fanta-headless` must not reach
GPUI (a `cargo tree | grep` in its `AGENTS.md`), and the model must not reach
Skia (a `cargo tree` count in the old engine repository,
[0003](0003-keep-skia-out-of-the-document-model.md)). CI ran neither.

Measured on 2026-10-27:

- The 15 engine crates depend on no editor or shell crate. The boundary that
  a separate engine repository needs is already clean, and nothing keeps it
  that way.
- `fig_viewer` holds 172,297 lines in 83 files, about 92,900 of them in
  inline test modules. `view.rs` alone has 23,910 lines and `document.rs`
  12,081. The document lifecycle (file watching, reconciliation, three-way
  merge, autosave) lives in this GPUI crate, where the cloud and the SDK
  cannot use it.
- About 16,000 lines of `fig_viewer` import no GPUI, workspace or settings
  crate: `layer_context_ops`, `measurements`, `export`, `instance_text`,
  `annotations`, `motion_edit`, `motion_stage`, `motion_video`, `clipboard`,
  `component_actions`, `prototype_player` and `draw_wand`.

## Decision

Adopt a ports-and-adapters layering, written down in
[`docs/architecture/layers.toml`](../architecture/layers.toml) and enforced
by `script/architecture check` in CI:

- Four groups: engine, editor, app (the composition root) and the inherited
  shell.
- Inside the engine, from the inside out: model; design-op vocabulary;
  interaction; source and import; rendering; playback; application; engine
  hosts. A layer may use the layers it lists and, transitively, what those may
  use.
- Skia is forbidden in the model, vocabulary, interaction, and source and
  import layers. `fanta-ui` is forbidden anywhere in the engine group,
  dev-dependencies included, and engine dev-dependencies stay inside the
  engine, so the engine's tests build without the editor.
- Anything else is a seam with a written reason. Today there are two: shell
  crates reach the editor only through `design_surface`, and `workspace`
  calls `fanta_format::is_project_dir` and `page_scope_of_source`.
- Changing a rule or adding a seam needs an ADR.

Grow the engine and keep the editor a host. New use-case logic goes into
`fanta-headless` or `fanta-api`, not `fig_viewer`. GPUI-free modules move
into the engine when a change touches them, and each move cites this record.

How the ports-and-adapters vocabulary maps onto the crates:

| Role | Crates and types |
| --- | --- |
| Domain | `fanta-doc`: `Doc`, `Scene`, `Operation`, history |
| Driving ports | `DesignOp` vocabulary (`fanta-design-ops`), `HeadlessProject` (`fanta-headless`), `DesignSession` (`fanta-api`), `DesignSurface` (`design_surface`) |
| Driven ports | `AssetResolver` and `DecodeListener` (render), `ImageStore` (headless), `ImageFetcher` (api), `FormatHandler` (format), `LayoutTree` (doc) |
| Driven adapters | `fanta-render` and `fanta-text` on Skia, `fanta-fnx` and `fanta-format` on files, `fanta-fig-interop` for `.fig` |
| Driving adapters | `fig_viewer` (desktop), `fanta-engine` (cloud worker), `fanta-runtime` (web and native players), `fanta-site` (website export) |

## Options considered

1. **A layer map with a CI check** (chosen). Rules live in one file, read by
   people, agents and the generator of the [crate map](../architecture/crate-map.md).
2. **`cargo-deny` bans.** Good for external crates, but it has no notion of
   layers inside the workspace.
3. **Tripwires in each crate's `AGENTS.md`.** The status quo: scattered, and
   CI runs none of them.
4. **Restructure into strict hexagonal crates now** (a ports crate, one
   adapter crate per technology). Large churn with no user-visible gain; done
   step by step instead, where a change needs it.

## Consequences

- Good: a dependency that breaks the architecture fails CI with a fix hint,
  the diagrams regenerate from the code, and the engine stays ready to move
  ([0024](0024-move-the-engine-into-its-own-repository.md)).
- Bad: one more check, and rule changes need a written decision. The layering
  is relaxed: an outer layer may use any inner one directly (`fig_viewer`
  uses `fanta-render`), which is how the code works today but is looser than
  textbook hexagonal design.
- Debt this record does not fix: the size of `fig_viewer`;
  `fanta-present` depending on `fanta-render`, which ties playback to one
  renderer; `fanta-text` mixing the rich-text model with Skia layout; and
  engine licence fields that disagree
  ([0026](0026-choose-the-engine-and-sdk-licence.md)).

## Confirmation

The Architecture workflow runs `script/architecture verify` on every pull
request, and `script/test-architecture` covers the checker itself.

## References

- [Architecture guide](../architecture/README.md), [crate map](../architecture/crate-map.md)
- Spec [0001](../../specs/0001-architecture-map-and-boundaries/spec.md)
