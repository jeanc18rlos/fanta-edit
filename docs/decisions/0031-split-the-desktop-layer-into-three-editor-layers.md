---
status: proposed
date: 2026-11-12
decision-makers: Jean Rojas
tags: [architecture, editor]
---

# Split the desktop layer into three editor layers

## Context

[ADR 0023](0023-enforce-layered-crate-boundaries.md) gave the engine eight
layers and the editor one. The `desktop` layer holds `fig_viewer`,
`design_surface`, `fanta_ui` and `fanta_languages`, and may use the whole
engine application layer and the whole inherited shell. Inside it the
checker allows any dependency in any direction.

The four crates are not peers. Measured on `main` at 9716f162 (2026-11-12)
from `cargo metadata`, workspace dependencies only:

| Crate | Depends on (editor) | Depends on (engine) | Depends on (shell) |
| --- | --- | --- | --- |
| `design_surface` | none | `fanta-design-ops` | none |
| `fanta_ui` | none | none | `assets`, `picker`, `settings`, `theme_settings` |
| `fanta_languages` | none | none | `grammars`, `language` |
| `fig_viewer` | `design_surface`, `fanta_ui` | ten engine crates | fifteen shell crates, seven more in tests |

Two properties hold today and nothing protects them:

- `design_surface` is the seam that five shell crates (`agent`,
  `agent_servers`, `agent_ui`, `workspace`, `project_panel`) reach the
  canvas through ([ADR 0007](0007-give-agents-a-live-design-seam-and-mcp-server.md)).
  It depends on the design-op vocabulary and nothing else. If it gained a
  dependency on a shell crate, the shell would depend on itself through the
  seam; if it gained one on `fig_viewer`, the seam would stop being a seam.
- `fanta_ui` and `fanta_languages` do not know the canvas or the engine.
  `fanta_languages` is used only by the `fanta` binary.

The founder decided on 2026-11-12 to improve structure step by step, with
editor layers and no big-bang crate split
([spec 0013](../../specs/0013-split-the-editors-largest-files-and-cap-file-size/spec.md)
covers the file sizes).

## Decision

Replace the `desktop` layer in `layers.toml` with three layers in the
editor group, along the direction the dependencies already have, moving no
code:

| Layer | Crates | May use |
| --- | --- | --- |
| `seam` | `design_surface` | `vocabulary` |
| `editor-ui` | `fanta_ui`, `fanta_languages` | `shell` |
| `desktop` | `fig_viewer` | `seam`, `editor-ui`, `application`, `shell` |

The `app` layer keeps `may_use = ["desktop", "hosts"]` and reaches
`fanta_languages` through `desktop`. The `shell` → `design_surface` seam
stays as written.

GPUI-free logic that leaves `fig_viewer` goes down into an engine crate
([ADR 0023](0023-enforce-layered-crate-boundaries.md),
[ADR 0024](0024-move-the-engine-into-its-own-repository.md) §5), not
sideways into a new editor crate.

## Options considered

1. **Three layers along today's dependencies** (chosen). It costs one edit
   to `layers.toml` and the regenerated crate map, breaks nothing, and
   turns two properties the code already has into rules.
2. **Keep one layer.** Nothing to do, and the seam stays protected only by
   a rules file that asks agents to be careful.
3. **Split `fig_viewer` into crates first** (canvas, panels, document
   lifecycle) and layer those. It is the larger gain and the larger cost:
   it conflicts with every open editor pull request. The founder ruled out
   a big-bang split; the layers can be refined when a crate is carved out.
4. **One chain with `fanta_ui` under `design_surface`.** The plan's
   shorthand read that way, but `design_surface` does not depend on
   `fanta_ui` and should not: the seam carries no UI.

## Consequences

- Good: `design_surface` can no longer depend on the shell, the engine
  application layer or the canvas without a decision. `fanta_ui` cannot
  reach the engine or the canvas. The crate map shows the editor's shape.
- Bad: three layers to read where there was one. A new editor crate must
  be placed in one of them.
- Bad: `desktop` still may use the whole shell. Narrowing that needs the
  shell to have layers of its own, which this record does not attempt.
- Follow-up: after acceptance, one pull request edits `layers.toml`, runs
  `script/architecture map` and updates the architecture guide's layer
  list and `design_surface/AGENTS.md`.

## Confirmation

`script/architecture check` passes with the new layers and no new seam.
`script/test-architecture` gains a case in which `design_surface` depends
on a shell crate and the check fails.

## References

- `docs/architecture/layers.toml`, [crate map](../architecture/crate-map.md)
- Spec [0013](../../specs/0013-split-the-editors-largest-files-and-cap-file-size/spec.md)
- Founder's plan of 2026-11-12, "Editor-group layers (ADR B, proposed)"
