# Design: Architecture map and boundaries

## Approach

- `docs/architecture/layers.toml` declares groups (engine, editor, app,
  shell), layers with `may_use` lists, forbidden external families, seams
  and external package families. The shell layer takes every crate no other
  layer lists, so a new Zed crate needs no entry and a new Fanta crate must
  choose a layer.
- `script/architecture` (Python standard library only, like the other
  scripts under `script/`) reads `cargo metadata --format-version 1 --no-deps`,
  which loads the manifests without resolving or building anything. It
  answers in well under a second here.
- `check` applies the rules. `may_use` is transitive. Seams match a crate or
  a layer on either side. Forbidden families are looked up over normal and
  build dependencies through workspace crates; for the engine group, whose
  dev-dependencies must stay inside it, direct dev-dependencies count too.
- `map` writes `docs/architecture/crate-map.md` with two Mermaid diagrams
  (GitHub renders them) and three tables. The crate diagram collapses the
  shell into one node, and its edges are the transitive reduction of the
  graph without the shell, so a direct dependency such as `fig_viewer` on
  `design_surface` stays visible.
- `index` validates the front matter of every record and rewrites the tables
  between the `<!-- decisions:start -->` and `<!-- specs:start -->` markers.
  `new` copies the templates.
- `verify` runs `check`, `map --check` and `index --check`, which is what
  `.github/workflows/architecture.yml` calls.

## Interfaces and data

- `layers.toml` keys: `[[group]]` (`id`, `title`, `summary`,
  `dev_dependencies` = `any` or `group`, `forbid`), `[[layer]]` (`id`,
  `group`, `title`, `role`, `summary`, `crates` = a list or `"rest"`,
  `may_use`, `forbid`), `[[seam]]` (`from`, `to`, `reason`) and
  `[[external]]` (`id`, `title`, `summary`, `packages` with `*` wildcards).
- Every crate outside the shell needs a `description` in its `Cargo.toml`;
  the map prints it.
- ADR front matter: `status`, `date`, `recorded`, `decision-makers`,
  `consulted`, `informed`, `supersedes`, `superseded-by`, `tags`. Spec front
  matter: `status`, `date`, `owner`, `decisions`, `repositories`, `tags`.

## Alternatives

- `cargo-deny` covers external bans but not layers inside the workspace.
- `cargo-depgraph` (used by the inherited `script/crate-dep-graph`) draws
  Graphviz output, which GitHub does not render, and needs a binary install.
- A Rust `xtask` would need a build before the check could run.

## Risks

- `--no-deps` sees declared dependencies only. A forbidden package pulled in
  by an external crate would be missed. None does today; the full graph can
  be checked by hand with `cargo tree`.
- Mermaid layout of the crate diagram can get crowded as crates are added.
  The tables stay authoritative.

## Verification

`python3 script/test-architecture` (22 unit tests) and
`script/architecture verify` on this branch, in CI and locally.
