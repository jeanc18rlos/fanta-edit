# Vendored Fanta crates

The Fanta engine and UI-kit crates below are vendored into this repository so
the tree builds standalone and the GPL-3.0 corresponding-source offer for the
shipped binary is satisfied by this repository alone.

| Crate(s) | Upstream | Commit |
|---|---|---|
| `fanta-canvas`, `fanta-doc`, `fanta-fig-interop`, `fanta-fnx`, `fanta-format`, `fanta-present`, `fanta-render`, `fanta-text`, `fanta-tools` | `squidred-dev/fantaisa-engine` | `9370fa2` |
| `fanta-gpui` | `squidred-dev/fanta-ui` | `a5edcd6` |

Upstream crates not vendored because nothing here uses them: `fanta-engine`,
`fanta-harness`, `fanta-psd-interop`, `fanta-illustrator-interop`,
`fanta-gpui-storybook`.

Changes made while vendoring, and nothing else:

- `repository`, `homepage`, `documentation` and `readme` were dropped from each
  `[package]` header; this workspace's `[workspace.package]` does not define
  them. `version`, `rust-version` and `license` were added to
  `[workspace.package]` instead, so the headers still inherit.
- Each crate directory gained a `LICENSE-GPL` symlink, as `script/check-licenses`
  requires.

Divergences from upstream made *after* vendoring, to be re-applied on a resync:

- `fanta-format`: the `AGENTS_MD` seed constant in
  `src/project/layout.rs` was rewritten for this product. The upstream text
  documented a `fanta-harness` CLI that Fanta does not ship; the Fanta text
  documents the live MCP server instead (`fanta --mcp-stdio`, the
  `fanta_live_mcp.json` discovery file, and the five canvas tools), plus the
  save/reload timing an agent needs before committing. The `.fnx` authoring
  guidance around it is upstream's, corrected where the parser had moved on.
  Nothing else in `fanta-format` diverges. A resync must keep the Fanta text:
  the constant is the only source of the file every project scaffolds, and a
  test pins the seeded file to it byte-for-byte.
- `gpui_component`: `src/webview.rs` lost its `use crate::PixelsExt;`. This
  workspace's `gpui` fork gives `Pixels` an inherent `as_f32`, which shadows the
  trait method the import existed to bring into scope, so the import is dead
  here and `script/clippy` (`--all-features`, which turns the `webview` feature
  on) rejects it. Behaviour is unchanged: an inherent method already wins over a
  trait method. Re-apply on a resync only if the fork drops `Pixels::as_f32`.

- `fanta-doc`: `tests/seam.rs` gained an `#[allow(clippy::disallowed_methods)]`
  on `skia_seam_is_enforced`. This workspace's `clippy.toml` denies
  `std::process::Command::output`, which that test uses to shell out to
  `cargo tree`; upstream has no such lint. Test-only, no behaviour change, and
  it never enters the shipped binary. Drop it if a resync brings a `clippy.toml`
  that permits the call.

- `fanta-fnx`: `serde_json` is built with the `float_roundtrip` feature
  (`Cargo.toml`). Upstream took serde_json's default parser, which is accurate
  only to within 1 ULP, so re-reading a printed `.fnx` moved every float that
  needs 17 significant digits — `21.762165069580078` came back as
  `21.76216506958008` — and the first save after a `.fig` import rewrote the
  whole page (25,311 insertions / 25,310 deletions, measured). A
  design-as-source format has to survive text → memory → text byte-identically.
  Keep on a resync; `src/tests.rs::seventeen_digit_floats_survive_the_text_boundary`
  fails without it.

- `fanta-fnx`: `src/sugar.rs` no longer blocks shape sugar on the mere PRESENCE
  of `local_size`. The attribute was removed from `SUGAR_BLOCKING_ATTRS` and is
  now checked by value in `sugared_form`: a viewport exactly equal to the
  regenerated shape's extent crops nothing, so the node re-sugars and the
  attribute rides along verbatim on the `<Rect>` / `<Ellipse>` spelling; any
  other viewport is a real crop and still keeps the node canonical. Upstream's
  blanket block meant the app's own `backfill_vector_viewports` turned every
  readable `<Rect width height />` into a `<Vector local_size path={…} />` blob
  on the first reload, permanently, with a spurious diff on every rectangle.
  Keep on a resync; guarded by `vector_with_viewport_equal_to_its_extent_resugars`
  and `vector_with_cropping_viewport_does_not_resugar`.

- `fanta-format`: `serde_json` is built with the `float_roundtrip` feature
  (`Cargo.toml`), for the same reason as `fanta-fnx` — the projected JSON files
  must reload to the exact floats they were written from. Guarded by
  `project::tests::reload_then_rewrite_is_byte_identical_for_f32_widened_floats`.

- `fanta-format`: `ProjectManifest` (`src/project/layout.rs`) dropped its
  `modified_at` field, so `fanta.json` no longer changes on every save. Nothing
  reads it — the reader uses only `format`, `version`, `project_id` and
  `schema_version`, and the authoritative timestamp lives in
  `doc/metadata.json`, which the 3-way merge tie-breaks on. It existed only to
  put a fourth file in the git diff of a one-node edit. Older trees still open
  (serde ignores the extra field); guarded by
  `project::layout::tests::manifest_from_an_older_build_still_opens`.

- `fanta-format`: `src/project/fnx.d.ts` documents an optional `local_size` on
  `FnxShapeSugarProps`, because the printer now emits it on a `<Rect>` /
  `<Ellipse>` whose viewport matches its extent (see the `fanta-fnx` sugar
  divergence above).

All other source is byte-identical to upstream. Edits belong here now; the
sibling checkouts are no longer part of the build.

## Third-party crates

`gpui_component` is not ours: it is [`longbridge/gpui-component`][gpui-component],
Apache-2.0, Copyright 2024 - 2025 Longbridge. Unlike the Fanta crates above it
keeps its own `LICENSE-APACHE` as a real file rather than a symlink, because
Apache-2.0 section 4 requires retaining the upstream copyright notice and this
repository's own `LICENSE-APACHE` carries a different one. `script/check-licenses`
knows about this through its `vendored_license_dirs` list and rejects turning
that file back into a symlink.

[gpui-component]: https://github.com/longbridge/gpui-component
