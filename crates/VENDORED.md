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

All source is otherwise byte-identical to upstream. Edits belong here now; the
sibling checkouts are no longer part of the build.
