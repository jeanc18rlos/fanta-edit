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
